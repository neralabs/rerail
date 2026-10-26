use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{FileScopeSlot, TABLE_BUILDS, UtilLimit};
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

fn python(ast: Value) -> CompiledCodeRuleDsl {
    compile("python", ast).unwrap_or_else(|error| panic!("{}", error.reason))
}

fn typescript(ast: Value) -> CompiledCodeRuleDsl {
    compile("typescript", ast).unwrap_or_else(|error| panic!("{}", error.reason))
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

#[test]
fn global_matches_python_references_no_binding_shadows() {
    let rule = python(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["print", "len", "hasattr", "Exception"] } } }),
    );
    let source = "\
print(len(x))
def f(len):
    return len(y)
def hasattr(a, b): pass
hasattr(o, 'k')
obj.print
print = log
print(z)
try:
    pass
except Exception:
    pass
Exception = ValueError
raise Exception
def g():
    return hasattr, print
";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["1:print", "1:len", "11:Exception"],
        "{result}"
    );
}

#[test]
fn global_matches_typescript_references_in_their_namespace() {
    let rule = typescript(
        json!({ "rule": { "resolves": { "global": ["alert", "Window", "undefined", "name"] } } }),
    );
    let source = "\
alert(1);
function f(alert: number) { return alert; }
let w: Window = window;
interface Window { x: number }
const o = { alert, name: 1 };
let u = undefined;
obj.alert;
declare global { var name: string; }
name;
";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    // `Window` is a local interface here; `name` stays global under `declare global`.
    assert_eq!(
        found(&result, source),
        ["1:alert", "5:alert", "6:undefined", "9:name"],
        "{result}"
    );
}

#[test]
fn negation_holds_for_shadowed_references_and_non_references() {
    let rule = python(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "kind": "identifier", "regex": "^open$", "not": { "resolves": { "global": ["open"] } } } },
    }));
    let source = "open(a)\ndef f(open):\n    open(b)\nimport io\nopen = io.open\nopen(c)\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["3:open(b)", "6:open(c)"],
        "{result}"
    );
}

#[test]
fn resolves_composes_with_constraints_relations_and_counts() {
    let rule = python(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "resolves": { "global": ["eval", "exec"] } } },
    }));
    let source = "eval(a)\nexec(b)\nobj.eval(c)\ndef f(eval):\n    eval(d)\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["1:eval(a)", "2:exec(b)"],
        "{result}"
    );

    let rule = python(json!({ "rule": {
        "kind": "function_definition",
        "has": { "stopBy": "end", "kind": "identifier", "resolves": { "global": ["input"] } },
    } }));
    let source = "def f():\n    return input()\ndef g(input):\n    return input()\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(found(&result, source).len(), 1, "{result}");

    let rule = typescript(json!({ "rule": {
        "kind": "program",
        "count": { "has": { "stopBy": "end", "resolves": { "global": ["confirm"] } }, "min": 2 },
    } }));
    let source = "confirm(1);\nfunction f(confirm) { confirm(2); }\n";
    assert_violations(&evaluate_file(&rule, "typescript", "a.ts", source), 0);
    let source = "confirm(1);\nconfirm(2);\n";
    assert_violations(&evaluate_file(&rule, "typescript", "a.ts", source), 1);
}

#[test]
fn eslint_global_comments_count_only_with_the_opt_in() {
    let source = "/* global jQuery, alert: off */\njQuery(1);\nalert(2);\nconfirm(3);\n";
    let plain = typescript(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["alert", "confirm"] } } }),
    );
    let result = evaluate_file(&plain, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["3:alert", "4:confirm"], "{result}");
    let honored = typescript(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["alert", "confirm"], "directives": "eslint" } } }),
    );
    let result = evaluate_file(&honored, "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        ["2:jQuery", "4:confirm"],
        "{result}"
    );
    // A comment declares a global; a local binding still shadows it.
    let source = "/* global jQuery */\nfunction f(jQuery) { jQuery(1); }\n";
    let result = evaluate_file(&honored, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
}

#[test]
fn malformed_resolves_are_rejected() {
    for (ast, expected) in [
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": [] } } }),
            "/clauses/0/forbid/ast/rule/resolves/global must not be empty",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["a.b"] } } }),
            "/clauses/0/forbid/ast/rule/resolves/global/0: `a.b` is not an identifier of at most 256 bytes",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["1x"] } } }),
            "/clauses/0/forbid/ast/rule/resolves/global/0: `1x` is not an identifier of at most 256 bytes",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": "print" } } }),
            "/clauses/0/forbid/ast/rule/resolves/global must be true or an array of names",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": {} } }),
            "/clauses/0/forbid/ast/rule/resolves must list `global` names, `import` names, `imported` names, `this`, `unresolved`, a `binding`, `write`, or `beforeLocal`",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["a"], "scope": "file" } } }),
            "unknown field `scope` at /clauses/0/forbid/ast/rule/resolves",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["a"], "directives": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/directives must be \"eslint\" (honor /* global */ comments)",
        ),
        (
            json!({ "rule": { "kind": "identifier", "not": { "resolves": { "global": ["a"], "directives": "eslint" } } } }),
            "/clauses/0/forbid/ast/rule/not/resolves/directives: /* global */ comments are ESLint's; Python has none",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "import": ["os..path"] } } }),
            "/clauses/0/forbid/ast/rule/resolves/import/0: `os..path` is not a dotted name (identifiers joined by `.`), a Rust path (identifiers joined by `::`) or a PHP name (labels joined by `\\`) of at most 256 bytes",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "import": ["os"], "directives": "eslint" } } }),
            "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments only add `global` names; list some",
        ),
    ] {
        assert_eq!(rejection(compile("python", ast.clone())), expected, "{ast}");
    }
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "resolves": { "import": ["react.useState"] } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/import: import resolution is Python-only for now; TypeScript rules can use `global`"
    );
    let many = (0..1025)
        .map(|index| format!("n{index}"))
        .collect::<Vec<_>>();
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "rule": { "resolves": { "global": many } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/global lists 1025 names; at most 1024 are allowed"
    );
}

#[test]
fn files_with_syntax_errors_are_source_incomplete() {
    let rule =
        python(json!({ "rule": { "kind": "identifier", "resolves": { "global": ["print"] } } }));
    // The error region could hide `def print`: the rule is not evaluated around it.
    let result = evaluate_file(&rule, "python", "a.py", "print(1)\ndef (:\n");
    assert_eq!(result["violations"], json!([]), "{result}");
    assert_eq!(
        result["failures"][0]["code"], "source-incomplete",
        "{result}"
    );
    let grammar = &grammar_matchers(&rule)[0];
    assert!(!grammar.sound_under_errors);
}

#[test]
fn imported_reads_only_its_import_statement_so_errors_elsewhere_leave_it_partial() {
    let sound = |resolves: Value| {
        grammar_matchers(&python(
            json!({ "rule": { "kind": "dotted_name", "resolves": resolves } }),
        ))
        .iter()
        .all(|grammar| grammar.sound_under_errors)
    };
    // `imported` builds no table and ignores the module path, with or without `relative: "text"`.
    assert!(sound(json!({ "imported": ["pdb"] })));
    assert!(sound(
        json!({ "imported": ["pdb"], "relative": "text", "prefix": true })
    ));
    // Anything that reads bindings stays incomplete, alone or beside `imported`.
    assert!(!sound(json!({ "global": ["print"] })));
    assert!(!sound(json!({ "import": ["pdb.set_trace"] })));
    assert!(!sound(json!({ "imported": ["pdb"], "global": ["print"] })));
    assert!(!sound(json!({ "imported": ["pdb"], "import": ["pdb"] })));
    // Negated, a statement broken before the name would read as "not imported": incomplete.
    let negated = python(
        json!({ "rule": { "kind": "dotted_name", "not": { "resolves": { "imported": ["pdb"] } } } }),
    );
    assert!(!grammar_matchers(&negated)[0].sound_under_errors);

    // F407's shape: names imported from `__future__`, relative ones by their text.
    let rule = python(json!({ "rule": {
        "kind": "dotted_name",
        "inside": { "kind": "import_from_statement", "has": { "kind": "relative_import", "has": {
            "kind": "dotted_name", "resolves": { "imported": ["__future__"], "relative": "text" } } } },
    } }));
    let source = "from .__future__ import nope\ndef (:\nfrom .__future__ import other\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    let texts = matched_texts(&result, source);
    assert!(texts.contains(&"nope".to_owned()), "{result}");
    assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
}

#[test]
fn imported_reads_nothing_through_an_error_before_the_name() {
    let rule = python(json!({ "rule": {
        "kind": "dotted_name",
        "resolves": { "imported": ["os.x", "a.b.x", "a.x"], "relative": "text" },
    } }));
    // Tree-sitter keeps `os` as the module and the broken `.$` as an ERROR sibling, and drops the
    // stray dot inside `a..b`: neither statement imports what its text would read as.
    for source in [
        "from os.$ import x\n",
        "from .a..b import x\n",
        "from a.1 import x\n",
    ] {
        let result = evaluate_file(&rule, "python", "a.py", source);
        assert_eq!(result["violations"], json!([]), "{source}: {result}");
    }
    let source = "from os import x  # clean text keeps the error region under half the file\nfrom os import y, $\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(matched_texts(&result, source), ["x"], "{result}");
}

#[test]
fn a_table_over_budget_fails_the_file_instead_of_reading_as_shadowed() {
    let source = "x = 1\n".repeat(20) + "print(x)\n";
    let root = GrammarDialect::Python.ast_grep(source.as_str());
    let print = root
        .root()
        .dfs()
        .find(|node| node.kind() == "identifier" && node.text() == "print")
        .expect("print");
    for rule in [
        json!({ "kind": "identifier", "resolves": { "global": ["print"] } }),
        json!({ "kind": "identifier", "regex": "^print$", "not": { "resolves": { "global": ["print"] } } }),
    ] {
        let compiled = python(json!({ "rule": rule }));
        let grammar = &grammar_matchers(&compiled)[0];
        // The table takes more nodes than the clause has left in this file.
        let mut work = 10;
        let _slot = FileScopeSlot::install(Vec::new(), None, false);
        assert_eq!(
            grammar.match_root(print.clone(), &mut work).err(),
            Some(UtilLimit::Scope),
            "{rule}"
        );
        // Enough budget: the same root resolves, and the build is charged to the clause.
        let mut work = 10_000;
        let matched = grammar
            .match_root(print.clone(), &mut work)
            .expect("within budget");
        assert_eq!(matched.is_some(), rule.get("not").is_none(), "{rule}");
        assert!(work < 10_000);
    }
}

#[test]
fn tables_are_built_lazily_once_per_file() {
    let source = "print(1)\nprint(2)\nlen(3)\n";
    TABLE_BUILDS.set(0);
    let without = python(json!({ "rule": { "kind": "identifier", "regex": "^print$" } }));
    assert_violations(&evaluate_file(&without, "python", "a.py", source), 2);
    assert_eq!(
        TABLE_BUILDS.get(),
        0,
        "a rule without resolves builds nothing"
    );
    // No listed name occurs: the name test rejects every node before a table is needed.
    let absent =
        python(json!({ "rule": { "kind": "identifier", "resolves": { "global": ["exit"] } } }));
    assert_violations(&evaluate_file(&absent, "python", "a.py", source), 0);
    assert_eq!(TABLE_BUILDS.get(), 0);
    let present = python(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["print", "len"] } } }),
    );
    assert_violations(&evaluate_file(&present, "python", "a.py", source), 3);
    assert_eq!(
        TABLE_BUILDS.get(),
        1,
        "one build serves every root of the file"
    );
}

#[test]
fn import_matches_chains_whose_head_an_import_binds() {
    let rule = python(json!({ "rule": {
        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
        "resolves": { "import": ["subprocess.run", "os.path.join", "time.sleep"] },
    } }));
    let source = "\
import subprocess
import subprocess as sp
from subprocess import run
from subprocess import run as r
import os.path
import os as o
from os import path
from . import time
from time import *
subprocess.run(a)
sp.run(b)
run(c)
r(d)
os.path.join(e)
o.path.join(f)
path.join(g)
time.sleep(h)
sleep(i)
subprocess.call(j)
def k():
    import time
    time.sleep(1)
subprocess = None
subprocess.run(l)
";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "10:subprocess.run",
            "11:sp.run",
            "12:run",
            "13:r",
            "14:os.path.join",
            "15:o.path.join",
            "16:path.join",
            "22:time.sleep",
        ],
        "{result}"
    );
}

#[test]
fn import_matches_rust_paths_through_use_declarations() {
    let rule = compile(
        "rust",
        json!({ "rule": {
            "any": [{ "kind": "identifier" }, { "kind": "scoped_identifier" }],
            "resolves": { "import": ["std::process::exit", "crate::a::helper", "regex::Regex"] },
        } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "\
use std::process;
use std::process::exit;
use std::process::exit as quit;
use self::inner::helper as near;
use super::a::helper as far;
use crate::a::helper;
extern crate regex as re;
mod inner {
    pub fn helper() {}
}
fn main() {
    std::process::exit(1);
    ::std::process::exit(2);
    process::exit(3);
    exit(4);
    quit(5);
    near();
    far();
    helper();
    crate::a::helper();
    re::Regex::new(\"\");
    std::process::abort();
    let exit = 6;
    exit;
    other::exit(7);
}
";
    // `src/a/b.rs` is the module `crate::a::b`: `self::inner` is `crate::a::b::inner`, and
    // `super::a` names `crate::a::a`; neither is listed.
    let result = evaluate_file(&rule, "rust", "src/a/b.rs", source);
    assert_eq!(
        found(&result, source),
        [
            "12:std::process::exit",
            "13:::std::process::exit",
            "14:process::exit",
            "15:exit",
            "16:quit",
            "19:helper",
            "20:crate::a::helper",
            "21:re::Regex",
        ],
        "{result}"
    );
    let rule = compile(
        "rust",
        json!({ "rule": {
            "kind": "identifier",
            "resolves": { "import": ["crate::a::b::inner::helper", "crate::a::a::helper"] },
        } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&rule, "rust", "src/a/b.rs", source);
    assert_eq!(found(&result, source), ["17:near", "18:far"], "{result}");
    // A file outside a crate's `src/` is a crate root of its own: `self::inner` is
    // `crate::inner`, and it has no `super`.
    let result = evaluate_file(&rule, "rust", "b.rs", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    for (import, expected) in [
        (
            json!(["std.process.exit"]),
            "/clauses/0/forbid/ast/rule/resolves/import/0: `std.process.exit` is a dotted name; Rust paths join identifiers with `::`",
        ),
        (
            json!(["self::a"]),
            "/clauses/0/forbid/ast/rule/resolves/import/0: `self::a` starts at a module; a listed path starts at a crate (`crate::`, `std::`), and reads through `self::` and `super::` are normalised to `crate::` paths",
        ),
    ] {
        let error = compile(
            "rust",
            json!({ "rule": { "kind": "identifier", "resolves": { "import": import } } }),
        )
        .err()
        .map(|error| error.reason);
        assert_eq!(error.as_deref(), Some(expected));
    }
    let error = compile(
        "python",
        json!({ "rule": { "kind": "identifier", "resolves": { "import": ["os::path"] } } }),
    )
    .err()
    .map(|error| error.reason);
    assert_eq!(
        error.as_deref(),
        Some(
            "/clauses/0/forbid/ast/rule/resolves/import/0: `os::path` is a Rust path; Python imports are dotted names"
        )
    );
}

#[test]
fn import_follows_ruff_ordering_and_scopes() {
    // B911: a use before the import is not the import.
    let rule = python(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "resolves": { "import": ["itertools.batched"] } } },
    }));
    let source = "batched(a)\nfrom itertools import batched\nbatched(b)\ndef f():\n    return batched(c)\ndef g(batched):\n    return batched(d)\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["3:batched(b)", "5:batched(c)"],
        "{result}"
    );
}

#[test]
fn either_key_may_match() {
    let rule = python(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "resolves": { "global": ["open"], "import": ["io.open", "codecs.open"] } } },
    }));
    let source = "import io, codecs\nopen(a)\nio.open(b)\ncodecs.open(c)\nos.open(d)\ndef f(open):\n    open(e)\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["2:open(a)", "3:io.open(b)", "4:codecs.open(c)"],
        "{result}"
    );
}

#[test]
fn a_global_statement_hides_nothing_outside_its_own_scope() {
    // Ruff 0.16.9 flags all three calls (ASYNC251) and `a`, `b` and the module (ASYNC250).
    let rule = python(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "resolves": { "import": ["time.sleep"], "global": ["input"] } } },
    }));
    let source = "\
import time
async def a():
    time.sleep(1)
    input()
async def g():
    global time, input
    time.sleep(1)
    input()
async def b():
    time.sleep(1)
    input()
input()
";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "3:time.sleep(1)",
            "4:input()",
            "7:time.sleep(1)",
            "10:time.sleep(1)",
            "11:input()",
            "12:input()",
        ],
        "{result}"
    );
    let rule =
        python(json!({ "rule": { "kind": "identifier", "resolves": { "global": ["set"] } } }));
    let source = "def g():\n    global set\n    set = 1\nx == set\ndef h():\n    return x == set\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(found(&result, source), ["4:set", "6:set"], "{result}");
    let rule = python(
        json!({ "rule": { "kind": "attribute", "resolves": { "import": ["numpy.dtype"] } } }),
    );
    let source =
        "import numpy\ndef g():\n    global numpy\n    numpy = None\nnumpy.dtype(x) == int\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(found(&result, source), ["5:numpy.dtype"], "{result}");
}

#[test]
fn parentheses_are_transparent() {
    // Ruff's AST has no parentheses: `(time).sleep` is `Attribute(Name time)`.
    let rule = python(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "resolves": { "import": ["time.sleep"], "global": ["input"] } } },
    }));
    let source = "\
import time
(time).sleep(1)
(time.sleep)(2)
((time)).sleep(3)
(
    time  # why
).sleep(4)
(input)(a)
((input))(b)
def f(input, time):
    (input)(c)
    (time).sleep(5)
";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "2:(time).sleep(1)",
            "3:(time.sleep)(2)",
            "4:((time)).sleep(3)",
            "5:(\n    time  # why\n).sleep(4)",
            "8:(input)(a)",
            "9:((input))(b)",
        ],
        "{result}"
    );
    let rule = typescript(json!({
        "rule": { "pattern": "$F($$$)" },
        "constraints": { "F": { "resolves": { "global": ["alert"] } } },
    }));
    let source = "(alert)(a);\n((alert))(b);\nfunction f(alert) { (alert)(c); }\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        ["1:(alert)(a)", "2:((alert))(b)"],
        "{result}"
    );
    let rule = typescript(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["console"] } } }),
    );
    let source = "(console).log(a);\nfunction f(console) { (console).log(b); }\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["1:console"], "{result}");
}

#[test]
fn an_except_name_ends_with_its_handler() {
    let rule = python(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["frozenset"] } } }),
    );
    let source =
        "try:\n    pass\nexcept Exception as frozenset:\n    frozenset\n\nx == frozenset\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(found(&result, source), ["6:frozenset"], "{result}");
}

#[test]
fn a_parameter_or_pattern_named_undefined_binds_it() {
    // typescript-eslint 8.70.1 resolves every use here to the local `undefined`.
    let rule = typescript(json!({ "rule": {
        "any": [{ "kind": "undefined" }, { "kind": "identifier" }],
        "resolves": { "global": ["undefined"] },
    } }));
    let source = "\
function f(undefined) { x === undefined }
function g([undefined]) { undefined }
function h({a: undefined}) { undefined }
class C { m(undefined) { undefined } }
{ const [undefined] = y; undefined }
{ const {a: undefined} = y; undefined }
undefined
";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["7:undefined"], "{result}");
}

#[test]
fn infer_binds_in_the_true_branch_only() {
    // typescript-eslint 8.70.1: a conditional type's scope holds its check, extends and true
    // types; the false branch is outside it.
    let rule = typescript(
        json!({ "rule": { "kind": "type_identifier", "resolves": { "global": ["Function"] } } }),
    );
    let source = "type T<X> = X extends infer Function ? 1 : Function\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["1:Function"], "{result}");
}

/// A TypeScript rule over identifiers resolving to `names`, with `options` as its languageOptions.
fn with_options(names: &[&str], options: Value) -> CompiledCodeRuleDsl {
    typescript(json!({
        "languageOptions": options,
        "rule": { "kind": "identifier", "resolves": { "global": names } },
    }))
}

#[test]
fn language_options_limit_global_to_the_declared_globals() {
    let names = ["Reflect", "String", "JSON", "require", "jQuery", "Temporal"];
    let source = "Reflect; String; JSON; require; jQuery; Temporal;\n";
    let run = |rule: &CompiledCodeRuleDsl| {
        found(&evaluate_file(rule, "typescript", "a.ts", source), source)
    };
    // Without options every listed, unshadowed reference matches (M7).
    let plain =
        typescript(json!({ "rule": { "kind": "identifier", "resolves": { "global": names } } }));
    assert_eq!(run(&plain).len(), 6);
    // ESLint's defaults: the latest ecmaVersion, a module.
    assert_eq!(
        run(&with_options(&names, json!({}))),
        ["1:Reflect", "1:String", "1:JSON", "1:Temporal"]
    );
    assert_eq!(
        run(&with_options(
            &names,
            json!({ "ecmaVersion": 5, "sourceType": "script" })
        )),
        ["1:String", "1:JSON"]
    );
    assert_eq!(
        run(&with_options(
            &names,
            json!({ "ecmaVersion": 3, "sourceType": "commonjs" })
        )),
        ["1:String", "1:require"]
    );
    // `6` is ES2015; `globals` adds names and turns builtins off, with ESLint's aliases.
    assert_eq!(
        run(&with_options(
            &names,
            json!({ "ecmaVersion": 6, "globals": { "String": "off", "jQuery": "readable", "Temporal": true } })
        )),
        ["1:Reflect", "1:JSON", "1:jQuery", "1:Temporal"]
    );
    assert_eq!(
        run(&with_options(
            &names,
            json!({ "ecmaVersion": "latest", "sourceType": "script", "globals": { "Reflect": false, "JSON": "off", "require": "writeable", "jQuery": null } })
        )),
        [
            "1:Reflect",
            "1:String",
            "1:require",
            "1:jQuery",
            "1:Temporal"
        ]
    );
    // A declared global is still shadowed by a file-local binding.
    let source = "function f(String) { String; }\nlet JSON = 1; JSON;\nString;\n";
    let result = evaluate_file(
        &with_options(&names, json!({ "ecmaVersion": 5, "sourceType": "script" })),
        "typescript",
        "a.ts",
        source,
    );
    assert_eq!(found(&result, source), ["3:String"], "{result}");
}

/// `compiled` over one file at `path`, of the dialect its suffix selects, with the host's
/// `package` (a root manifest of that `type`) where given.
fn evaluate_in_package(
    compiled: &CompiledCodeRuleDsl,
    path: &str,
    package_type: Option<&str>,
    source: &str,
) -> Value {
    let language = crate::code_rule_dsl::language::label_of_path(path).expect("a dialect file");
    let mut file = json!({ "language": language, "path": path, "source": source });
    if let Some(package_type) = package_type {
        file["package"] = json!({ "folder": "", "type": package_type });
    }
    let snapshot = json!({
        "facts": [], "files": [file], "format": "rerail.repository-snapshot/v1", "revision": 0,
    });
    let output = compiled
        .evaluate(snapshot.to_string(), "report".to_owned())
        .expect("evaluation succeeds");
    serde_json::from_str(&output).expect("evaluation is JSON")
}

#[test]
fn without_a_source_type_each_file_has_its_own_module_system() {
    let run = |rule: &CompiledCodeRuleDsl, path: &str, package: Option<&str>, source: &str| {
        found(&evaluate_in_package(rule, path, package, source), source)
    };
    let source = "require; module;\n";
    let both = ["1:require", "1:module"];
    let neither: [&str; 0] = [];
    let unpinned = with_options(&["require", "module"], json!({}));
    // ESLint's default config: `.cjs` is CommonJS, the rest modules, a `.js` without a manifest
    // included; TypeScript's `.cts` and `.mts` follow theirs.
    for path in ["a.cjs", "a.cts"] {
        assert_eq!(run(&unpinned, path, None, source), both, "{path}");
    }
    for path in ["a.mjs", "a.mts", "a.ts", "a.tsx", "a.js", "a.jsx"] {
        assert_eq!(run(&unpinned, path, None, source), neither, "{path}");
    }
    // A `.js` or `.ts` file follows its package's `type`, as Node runs it; the suffixes that name
    // a module system, `.tsx` (which Node does not run) and declaration files do not.
    assert_eq!(run(&unpinned, "a.js", Some("module"), source), neither);
    assert_eq!(run(&unpinned, "a.jsx", Some("commonjs"), source), both);
    assert_eq!(run(&unpinned, "a.mjs", Some("commonjs"), source), neither);
    assert_eq!(run(&unpinned, "a.cjs", Some("module"), source), both);
    assert_eq!(run(&unpinned, "a.ts", Some("commonjs"), source), both);
    assert_eq!(run(&unpinned, "a.ts", Some("module"), source), neither);
    for path in ["a.tsx", "a.mts", "a.d.ts", "a.d.css.ts", "a.d.mts"] {
        assert_eq!(
            run(&unpinned, path, Some("commonjs"), source),
            neither,
            "{path}"
        );
    }
    // A `.d.cts` declares a CommonJS module, as TypeScript reads it, whatever the package's type.
    assert_eq!(run(&unpinned, "a.d.cts", Some("module"), source), both);
    // Node strips types before it detects: what type stripping erases is no module syntax, and
    // TypeScript's CommonJS forms are none either.
    for (ts, line) in [
        ("import x from 'x';\n", false),
        ("import { type A } from 'x';\n", false),
        ("export const a: number = 1;\n", false),
        ("import type { A } from 'x';\n", true),
        ("export type { A };\n", true),
        ("export type A = number;\n", true),
        ("export interface A {}\n", true),
        ("export declare const a: number;\n", true),
        ("import x = require('x');\n", true),
        // An abstract class is a class once stripped, so redeclaring a wrapper name makes a
        // module; a `declare` one is erased with its name.
        ("abstract class require {}\n", false),
        ("abstract class Other {}\n", true),
        ("declare class __dirname {}\n", true),
        ("declare const __filename: string;\n", true),
    ] {
        let source = format!("{ts}require; module;\n");
        let expected: &[&str] = if line {
            &["2:require", "2:module"]
        } else {
            &[]
        };
        assert_eq!(
            run(&unpinned, "a.ts", Some("unset"), &source),
            expected,
            "{ts}"
        );
    }
    // Without a valid `type`, Node's detection: a static `import` or `export` declaration at the
    // top level, or `import.meta`, makes a module; a dynamic `import()` does not.
    assert_eq!(run(&unpinned, "a.js", Some("unset"), source), both);
    for (esm, line) in [
        ("import x from 'x';\n", false),
        ("export const a = 1;\n", false),
        ("function f() { return import.meta.url; }\n", false),
        ("import('x');\n", true),
        ("const exported = 'export default 1';\n", true),
        // Node also detects a top-level `await` and a lexical redeclaration of a CommonJS wrapper
        // name; an `await` inside a function is a CommonJS file's.
        ("await Promise.resolve();\n", false),
        (
            "let { __dirname: dir } = x;\nlet __filename = dir;\n",
            false,
        ),
        ("async function f() { await x; }\n", true),
    ] {
        let source = format!("{esm}require; module;\n");
        let expected: &[&str] = if line {
            &["2:require", "2:module"]
        } else {
            &[]
        };
        assert_eq!(
            run(&unpinned, "a.js", Some("unset"), &source),
            expected,
            "{esm}"
        );
    }
    // A `sourceType` the query pins wins over the file's.
    let pinned_module = with_options(&["require", "module"], json!({ "sourceType": "module" }));
    assert_eq!(run(&pinned_module, "a.cjs", None, source), neither);
    let pinned_commonjs = with_options(&["require", "module"], json!({ "sourceType": "commonjs" }));
    assert_eq!(run(&pinned_commonjs, "a.mjs", Some("module"), source), both);
    // Without `languageOptions`, `unresolved` reads the defaults, the file's own module system
    // included; `this` reads a CommonJS file's top level as no-eval does.
    let unresolved =
        typescript(json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": true } } }));
    assert_eq!(run(&unresolved, "a.cjs", None, source), neither);
    assert_eq!(run(&unresolved, "a.mjs", None, source), both);
    let this = typescript(json!({
        "languageOptions": {},
        "rule": { "kind": "this", "resolves": { "this": "global" } },
    }));
    assert_eq!(run(&this, "a.cjs", None, "this;\n"), ["1:this"]);
    assert_eq!(run(&this, "a.js", Some("unset"), "this;\n"), ["1:this"]);
    assert_eq!(run(&this, "a.mjs", None, "this;\n"), neither);
}

#[test]
fn lib_adds_typescript_eslint_lib_globals_in_their_namespaces() {
    // typescript-eslint 8.70.1's `dom` lib: `NodeListOf` is a type, `NodeList` a type and a value;
    // `Promise` comes from `es2015`, which `dom` includes. `document` is no lib variable.
    let source = "let a: NodeListOf<Node>; NodeListOf; let b: NodeList = NodeList; Promise; document; let c: Missing;\n";
    let unresolved = |options: Value| {
        let rule = typescript(json!({
            "languageOptions": options,
            "rule": { "any": [{ "kind": "identifier" }, { "kind": "type_identifier" }], "resolves": { "unresolved": true } },
        }));
        found(&evaluate_file(&rule, "typescript", "a.ts", source), source)
    };
    assert_eq!(
        unresolved(json!({})),
        [
            "1:NodeListOf",
            "1:Node",
            "1:NodeListOf",
            "1:NodeList",
            "1:NodeList",
            "1:document",
            "1:Missing"
        ]
    );
    assert_eq!(
        unresolved(json!({ "lib": ["dom"] })),
        ["1:NodeListOf", "1:document", "1:Missing"]
    );
    // Lower-cased as typescript-eslint does; a lib without `dom` adds none of its names.
    assert_eq!(
        unresolved(json!({ "lib": ["DOM"] })),
        ["1:NodeListOf", "1:document", "1:Missing"]
    );
    assert_eq!(
        unresolved(json!({ "lib": ["es2015"] })),
        unresolved(json!({}))
    );
    assert_eq!(unresolved(json!({ "lib": [] })), unresolved(json!({})));
    // `global` holds for a listed lib name in its namespace, unshadowed.
    let global = typescript(json!({
        "languageOptions": { "lib": ["dom"] },
        "rule": { "any": [{ "kind": "identifier" }, { "kind": "type_identifier" }], "resolves": { "global": ["NodeListOf", "NodeList"] } },
    }));
    let result = evaluate_file(&global, "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        ["1:NodeListOf", "1:NodeList", "1:NodeList"],
        "{result}"
    );
    // A `/* global */` comment's `off` and a configured `off` leave a lib variable in place.
    let source = "/* global NodeList: off */\nNodeList; Node;\n";
    let rule = typescript(json!({
        "languageOptions": { "lib": ["dom"], "globals": { "Node": "off" } },
        "rule": { "kind": "identifier", "resolves": { "unresolved": true, "directives": "eslint" } },
    }));
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    for (language, options, expected) in [
        (
            "typescript",
            json!({ "lib": ["dom", "esnext.nope"] }),
            "/clauses/0/forbid/ast/languageOptions/lib/1: `esnext.nope` is not a lib typescript-eslint 8.70.1 knows",
        ),
        (
            "typescript",
            json!({ "lib": "dom" }),
            "/clauses/0/forbid/ast/languageOptions/lib must be an array of lib names",
        ),
        (
            "python",
            json!({ "lib": ["dom"] }),
            "unknown field `lib` at /clauses/0/forbid/ast/languageOptions",
        ),
    ] {
        let ast = json!({ "languageOptions": options, "rule": { "kind": "identifier" } });
        assert_eq!(rejection(compile(language, ast.clone())), expected, "{ast}");
    }
}

#[test]
fn language_options_take_global_comments_only_with_the_opt_in() {
    let source = "/* global Reflect, jQuery, JSON: off */\nReflect; jQuery; JSON; Math; foo;\n";
    let options = json!({ "ecmaVersion": 5, "sourceType": "script" });
    let plain = with_options(&["Reflect", "JSON", "Math"], options.clone());
    let result = evaluate_file(&plain, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["2:JSON", "2:Math"], "{result}");
    let honored = typescript(json!({
        "languageOptions": options,
        "rule": { "kind": "identifier", "resolves": { "global": ["Reflect", "JSON", "Math"], "directives": "eslint" } },
    }));
    // The comments decide the names they mention; an unlisted name never matches.
    let result = evaluate_file(&honored, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["2:Reflect", "2:Math"], "{result}");
}

#[test]
fn declared_false_asks_only_whether_a_listed_name_is_shadowed() {
    // ESLint's no-alert tests shadowing, not existence: `window` counts at ES5 without browser
    // globals, and a name configured `off` still counts.
    let source = "window; alert; Reflect; function f(alert) { alert; }\nvar window;\n";
    let options =
        json!({ "ecmaVersion": 5, "sourceType": "script", "globals": { "alert": "off" } });
    let run = |resolves: Value| {
        let rule = typescript(json!({
            "languageOptions": options,
            "rule": { "kind": "identifier", "resolves": resolves },
        }));
        found(&evaluate_file(&rule, "typescript", "a.ts", source), source)
    };
    let names = json!(["window", "alert", "Reflect"]);
    assert_eq!(
        run(json!({ "global": names })),
        Vec::<String>::new(),
        "declared (the default under options)"
    );
    assert_eq!(
        run(json!({ "global": names, "declared": true })),
        Vec::<String>::new()
    );
    // `window` is shadowed by the script's `var window`; the others are not.
    assert_eq!(
        run(json!({ "global": names, "declared": false })),
        ["1:alert", "1:Reflect"]
    );
    // Python: a name that is no builtin of the version still counts when only shadowing is asked.
    let python = |declared: bool| {
        compile(
            "python",
            json!({
                "languageOptions": { "pythonVersion": "3.9" },
                "rule": { "kind": "identifier", "resolves": { "global": ["aiter", "print"], "declared": declared } },
            }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason))
    };
    let source = "aiter\nprint\n";
    let result = evaluate_file(&python(true), "python", "a.py", source);
    assert_eq!(found(&result, source), ["2:print"], "{result}");
    let result = evaluate_file(&python(false), "python", "a.py", source);
    assert_eq!(found(&result, source), ["1:aiter", "2:print"], "{result}");
}

#[test]
fn script_declarations_are_the_global_variable_with_the_opt_in() {
    let source = "\
var Array; Array;
function Object() {} Object;
let Map; class Set {} Map; Set;
function f(Array) { Array; Object; }
{ let Object; Object; }
Reflect;
";
    let rule = |source_type: &str, script_declarations: bool| {
        let mut resolves = json!({ "global": ["Array", "Object", "Map", "Set", "Reflect"] });
        if script_declarations {
            resolves["scriptDeclarations"] = json!(true);
        }
        typescript(json!({
            "languageOptions": { "ecmaVersion": 2015, "sourceType": source_type },
            "rule": { "kind": "identifier", "resolves": resolves },
        }))
    };
    let run = |rule: &CompiledCodeRuleDsl| {
        let result = evaluate_file(rule, "typescript", "a.js", source);
        found(&result, source)
    };
    // Off (the default): the declarations shadow.
    assert_eq!(run(&rule("script", false)), ["6:Reflect"]);
    // On: a script's top-level declarations are the global variable, declared or not; a
    // parameter or block binding still shadows.
    assert_eq!(
        run(&rule("script", true)),
        [
            "1:Array",
            "2:Object",
            "3:Map",
            "3:Set",
            "4:Object",
            "6:Reflect"
        ]
    );
    // CommonJS top-level declarations bind in a function scope: nothing changes.
    assert_eq!(run(&rule("commonjs", true)), ["6:Reflect"]);
    let module = typescript(json!({
        "languageOptions": { "sourceType": "module" },
        "rule": { "kind": "identifier", "resolves": { "global": ["Array", "Reflect"], "scriptDeclarations": true } },
    }));
    let source = "var Array; Array; Reflect;\n";
    let result = evaluate_file(&module, "typescript", "a.js", source);
    assert_eq!(found(&result, source), ["1:Reflect"], "{result}");
}

#[test]
fn this_resolves_to_the_global_object_at_the_top_level_of_a_script() {
    let rule = |source_type: &str| {
        typescript(json!({
            "languageOptions": { "sourceType": source_type },
            "rule": { "kind": "this", "resolves": { "this": "global" } },
        }))
    };
    let source = "\
this.eval(1);
(this).alert;
() => this;
function f() { this; }
const g = function () { this; };
class C extends this.Base {
  [this.key] = this.value;
  static { this; }
  [this.name]() { this; }
  get x() { return this; }
}
const o = { m() { this; }, p: this };
namespace N { this; }
if (a) { this; }
";
    let result = evaluate_file(&rule("script"), "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        [
            "1:this", "2:this", "3:this", "6:this", "7:this", "9:this", "12:this", "14:this"
        ],
        "{result}"
    );
    // ESLint's no-eval reads a CommonJS file's top level as a script's: its Program is no module.
    let result = evaluate_file(&rule("commonjs"), "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        [
            "1:this", "2:this", "3:this", "6:this", "7:this", "9:this", "12:this", "14:this"
        ],
        "{result}"
    );
    let result = evaluate_file(&rule("module"), "typescript", "a.ts", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    // Either key may match.
    let either = typescript(json!({
        "languageOptions": { "sourceType": "script", "ecmaVersion": 2020 },
        "rule": { "any": [{ "kind": "this" }, { "kind": "identifier" }], "resolves": { "this": "global", "global": ["globalThis", "window"] } },
    }));
    let source = "this.eval(1); globalThis.eval(2); window.eval(3);\n";
    let result = evaluate_file(&either, "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        ["1:this", "1:globalThis"],
        "{result}"
    );
}

#[test]
fn this_global_scope_is_where_eslint_scopes_global_scope_holds_it() {
    let rule = |options: Value| {
        typescript(json!({
            "languageOptions": options,
            "rule": { "kind": "this", "resolves": { "this": "globalScope" } },
        }))
    };
    let source = "\
this;
{ this; }
if (a) this;
switch (this) { case 1: this; }
try { this; } catch (e) { this; }
for (var i in this) this;
for (let j in this) this;
for (let k = this;;) this;
for (;;) this;
class C extends this {}
() => this;
function f() { this; }
label: this;
x = { a: this };
";
    let run = |options: Value| {
        let result = evaluate_file(&rule(options), "typescript", "a.js", source);
        found(&result, source)
    };
    assert_eq!(
        run(json!({ "ecmaVersion": 2015, "sourceType": "script" })),
        [
            "1:this", "3:this", "6:this", "6:this", "9:this", "13:this", "14:this"
        ]
    );
    // Before ES2015 blocks, switch statements and lexical loops are no scopes; catch is.
    assert_eq!(
        run(json!({ "ecmaVersion": 5, "sourceType": "script" })),
        [
            "1:this", "2:this", "3:this", "4:this", "4:this", "5:this", "6:this", "6:this",
            "7:this", "7:this", "8:this", "8:this", "9:this", "13:this", "14:this"
        ]
    );
    // A with statement is a scope from its object on.
    let source = "with (this) this;\nthis;\n";
    let result = evaluate_file(
        &rule(json!({ "ecmaVersion": 5, "sourceType": "script" })),
        "typescript",
        "a.js",
        source,
    );
    assert_eq!(found(&result, source), ["2:this"], "{result}");
    for source_type in ["module", "commonjs"] {
        let result = evaluate_file(
            &rule(json!({ "sourceType": source_type })),
            "typescript",
            "a.js",
            "this;\n",
        );
        assert_eq!(starts(&result).len(), 0, "{source_type}");
    }
}

#[test]
fn sloppy_functions_bind_this_to_the_global_object_by_strictness() {
    let rule = |options: Value, sloppy: bool| {
        let mut resolves = json!({ "this": "global" });
        if sloppy {
            resolves["sloppyFunctions"] = json!(true);
        }
        typescript(
            json!({ "languageOptions": options, "rule": { "kind": "this", "resolves": resolves } }),
        )
    };
    let source = "\
this;
function f() { this; }
function g() { 'use strict'; this; }
function h() { \"use strict\"; function i() { this; } }
function j() { ('use strict'); this; }
const k = function () { this; };
function* l() { this; }
const m = () => { 'use strict'; return function () { this; }; };
const n = () => this;
class A extends function () { this; } { o() { this; } static { this; } }
const p = { q() { this; }, r: function () { this; } };
function s() { f(); 'use strict'; this; }
";
    let run = |options: Value, sloppy: bool| {
        let result = evaluate_file(&rule(options, sloppy), "typescript", "a.js", source);
        found(&result, source)
    };
    let es5 = json!({ "ecmaVersion": 2015, "sourceType": "script" });
    // Off (the default): only the top level.
    assert_eq!(run(es5.clone(), false), ["1:this", "9:this"]);
    // An ordinary function without a 'use strict' prologue of its own or around it; methods,
    // classes and arrows are not ordinary functions (an arrow reads the outer `this`). A
    // property's function expression counts: a port tells default bindings apart.
    assert_eq!(
        run(es5, true),
        [
            "1:this", "2:this", "5:this", "6:this", "7:this", "9:this", "11:this", "12:this"
        ]
    );
    // A module, or impliedStrict, makes every function strict.
    assert_eq!(
        run(json!({ "sourceType": "module" }), true),
        Vec::<String>::new()
    );
    assert_eq!(
        run(
            json!({ "sourceType": "script", "parserOptions": { "ecmaFeatures": { "impliedStrict": true } } }),
            true
        ),
        ["1:this", "9:this"]
    );
    // ES3 has no strict mode: directives and impliedStrict change nothing.
    let source = "function f() { 'use strict'; this; }\n";
    for implied in [false, true] {
        let options = json!({ "ecmaVersion": 3, "sourceType": "script", "parserOptions": { "ecmaFeatures": { "impliedStrict": implied } } });
        let result = evaluate_file(&rule(options, true), "typescript", "a.js", source);
        assert_eq!(found(&result, source), ["1:this"], "{result}");
    }
    // globalReturn makes the top level a function: its `this` counts unless it is strict.
    let source = "this;\nfunction f() { this; }\n";
    let global_return = |implied: bool| json!({ "ecmaVersion": 5, "sourceType": "script", "parserOptions": { "ecmaFeatures": { "globalReturn": true, "impliedStrict": implied } } });
    let result = evaluate_file(
        &rule(global_return(false), false),
        "typescript",
        "a.js",
        source,
    );
    assert_eq!(found(&result, source), ["1:this"], "{result}");
    let result = evaluate_file(
        &rule(global_return(true), true),
        "typescript",
        "a.js",
        source,
    );
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    let strict = format!("'use strict';\n{source}");
    let result = evaluate_file(
        &rule(global_return(false), true),
        "typescript",
        "a.js",
        &strict,
    );
    assert_eq!(found(&result, &strict), Vec::<String>::new(), "{result}");
    // globalReturn moves the top level out of the global scope.
    let scope = typescript(json!({
        "languageOptions": global_return(false),
        "rule": { "kind": "this", "resolves": { "this": "globalScope" } },
    }));
    assert_eq!(
        starts(&evaluate_file(&scope, "typescript", "a.js", "this;\n")).len(),
        0
    );
    let declarations = typescript(json!({
        "languageOptions": global_return(false),
        "rule": { "kind": "identifier", "resolves": { "global": ["Array"], "scriptDeclarations": true } },
    }));
    let source = "var Array; Array;\n";
    let result = evaluate_file(&declarations, "typescript", "a.js", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
}

#[test]
fn malformed_language_options_are_rejected() {
    let rule = json!({ "kind": "identifier", "resolves": { "global": ["a"] } });
    for (options, expected) in [
        (
            json!([]),
            "/clauses/0/forbid/ast/languageOptions must be an object",
        ),
        (
            json!({ "parserOptions": [] }),
            "/clauses/0/forbid/ast/languageOptions/parserOptions must be an object",
        ),
        (
            json!({ "parserOptions": { "ecmaVersion": 5 } }),
            "unknown field `ecmaVersion` at /clauses/0/forbid/ast/languageOptions/parserOptions",
        ),
        (
            json!({ "parserOptions": { "ecmaFeatures": { "jsx": true } } }),
            "unknown field `jsx` at /clauses/0/forbid/ast/languageOptions/parserOptions/ecmaFeatures",
        ),
        (
            json!({ "parserOptions": { "ecmaFeatures": { "globalReturn": 1 } } }),
            "/clauses/0/forbid/ast/languageOptions/parserOptions/ecmaFeatures/globalReturn must be true or false",
        ),
        (
            json!({ "ecmaVersion": 4 }),
            "/clauses/0/forbid/ast/languageOptions/ecmaVersion must be \"latest\", 3, 5, 6 to 17, or 2015 to 2026",
        ),
        (
            json!({ "ecmaVersion": 2027 }),
            "/clauses/0/forbid/ast/languageOptions/ecmaVersion must be \"latest\", 3, 5, 6 to 17, or 2015 to 2026",
        ),
        (
            json!({ "ecmaVersion": "next" }),
            "/clauses/0/forbid/ast/languageOptions/ecmaVersion must be \"latest\", 3, 5, 6 to 17, or 2015 to 2026",
        ),
        (
            json!({ "ecmaVersion": 5 }),
            "/clauses/0/forbid/ast/languageOptions: sourceType \"module\" (the default) needs ecmaVersion 2015 or later; ESLint refuses it",
        ),
        (
            json!({ "sourceType": "esm" }),
            "/clauses/0/forbid/ast/languageOptions/sourceType must be \"script\", \"module\" or \"commonjs\"",
        ),
        (
            json!({ "globals": [] }),
            "/clauses/0/forbid/ast/languageOptions/globals must be an object",
        ),
        (
            json!({ "globals": { "a": "on" } }),
            "/clauses/0/forbid/ast/languageOptions/globals/a must be \"readonly\", \"writable\" or \"off\" (or an ESLint alias)",
        ),
        (
            json!({ "globals": { "a-b": "off" } }),
            "/clauses/0/forbid/ast/languageOptions/globals: `a-b` is not an identifier of at most 256 bytes",
        ),
    ] {
        let ast = json!({ "languageOptions": options, "rule": rule });
        assert_eq!(
            rejection(compile("typescript", ast.clone())),
            expected,
            "{ast}"
        );
    }
    // Sized for real environment unions (browser + node + ...), and still bounded.
    let globals = |count: usize| {
        (0..count)
            .map(|index| (format!("n{index}"), json!("readonly")))
            .collect::<serde_json::Map<_, _>>()
    };
    assert!(
        compile(
            "typescript",
            json!({ "languageOptions": { "globals": globals(16_384) }, "rule": rule })
        )
        .is_ok()
    );
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "languageOptions": { "globals": globals(16_385) }, "rule": rule })
        )),
        "/clauses/0/forbid/ast/languageOptions/globals lists 16385 names; at most 16384 are allowed"
    );
    for (ast, expected) in [
        (
            json!({ "rule": { "kind": "this", "resolves": { "this": "global" } } }),
            "/clauses/0/forbid/ast/rule/resolves/this: the global object depends on languageOptions.sourceType; declare languageOptions",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "this", "resolves": { "this": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/this must be \"global\" or \"globalScope\"",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "this", "resolves": { "this": "global", "directives": "eslint" } } }),
            "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments only add `global` names; list some",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "this", "resolves": { "this": "global", "sloppyFunctions": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/sloppyFunctions must be true",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "this", "resolves": { "this": "globalScope", "sloppyFunctions": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/sloppyFunctions: only `this: \"global\"` reads a function's `this`",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "identifier", "resolves": { "global": ["a"], "declared": "no" } } }),
            "/clauses/0/forbid/ast/rule/resolves/declared must be true or false",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "this", "resolves": { "this": "global", "declared": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/declared: only `global` names are declared; list some",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "identifier", "resolves": { "global": ["a"], "declared": false, "directives": "eslint" } } }),
            "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments decide what is declared, which `declared: false` ignores",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "identifier", "resolves": { "global": ["a"], "scriptDeclarations": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations must be true",
        ),
        (
            json!({ "languageOptions": {}, "rule": { "kind": "this", "resolves": { "this": "global", "scriptDeclarations": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: only `global` names resolve to declarations; list some",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["a"], "scriptDeclarations": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: whether a file is a script comes from languageOptions.sourceType; declare languageOptions",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["a"], "declared": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/declared: the declared globals come from languageOptions; declare languageOptions",
        ),
    ] {
        assert_eq!(
            rejection(compile("typescript", ast.clone())),
            expected,
            "{ast}"
        );
    }
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "rule": { "kind": "identifier", "resolves": { "this": "global" } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/this: `this` is TypeScript's; Python has none"
    );
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "languageOptions": {}, "rule": { "kind": "identifier", "resolves": { "global": ["a"], "scriptDeclarations": true } } })
        )),
        "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: scripts are ESLint's; Python has none"
    );
}

/// Evaluates one Python file whose snapshot entry carries `module` (when given).
fn evaluate_module(
    compiled: &CompiledCodeRuleDsl,
    path: &str,
    module: Option<&str>,
    source: &str,
) -> Value {
    evaluate_module_value(compiled, path, module.map(|module| json!(module)), source)
}

/// Evaluates one Python file whose snapshot entry carries `module` in either form: a dotted
/// string or a list of names.
fn evaluate_module_value(
    compiled: &CompiledCodeRuleDsl,
    path: &str,
    module: Option<Value>,
    source: &str,
) -> Value {
    let mut file = json!({ "language": "python", "path": path, "source": source });
    if let Some(module) = module {
        file["module"] = module;
    }
    let snapshot = json!({ "facts": [], "files": [file], "format": "rerail.repository-snapshot/v1", "revision": 0 });
    let output = compiled
        .evaluate(snapshot.to_string(), "report".to_owned())
        .expect("evaluation succeeds");
    serde_json::from_str(&output).expect("evaluation is JSON")
}

#[test]
fn a_definition_resolves_to_the_files_module_path() {
    let rule = python(json!({ "rule": {
        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
        "resolves": { "import": ["contextlib.suppress", "contextlib.Box.open", "pkg.contextlib.suppress", "pkg.contextlib.Box.open"] },
    } }));
    let source = "\
def suppress(*a): pass
class Box:
    def open(self): pass
with suppress(): pass
Box.open
def f():
    class Box: pass
    Box.open
    suppress
";
    // Ruff 0.16.9: outside a package the module is the file's stem, at any depth of the path.
    for path in ["contextlib.py", "fn/contextlib.py", "a/b/contextlib.pyi"] {
        let result = evaluate_module(&rule, path, None, source);
        assert_eq!(
            found(&result, source),
            ["4:suppress", "5:Box.open", "8:Box.open", "9:suppress"],
            "{path}: {result}"
        );
    }
    let result = evaluate_module(
        &rule,
        "src/pkg/contextlib.py",
        Some("pkg.contextlib"),
        source,
    );
    assert_eq!(
        found(&result, source),
        ["4:suppress", "5:Box.open", "8:Box.open", "9:suppress"],
        "{result}"
    );
    // The qualified names are the module path's, not the stem's.
    let stem_only = python(
        json!({ "rule": { "kind": "identifier", "resolves": { "import": ["contextlib.suppress"] } } }),
    );
    let result = evaluate_module(
        &stem_only,
        "pkg/contextlib.py",
        Some("pkg.contextlib"),
        source,
    );
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    // A stem that is no identifier qualifies nothing a code rule can list.
    let result = evaluate_module(&stem_only, "contextlib.v2.py", None, source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
}

#[test]
fn relative_imports_resolve_through_an_explicit_module_path() {
    let rule = python(json!({ "rule": {
        "kind": "attribute",
        "resolves": { "import": ["pkg.sub.sibling.attr", "pkg.top.attr", "beyond.attr", "pkg.sub.sibling.thing.attr", "pkg.x.attr"] },
    } }));
    let source = "\
from . import sibling
sibling.attr
from .. import top
top.attr
from ... import beyond
beyond.attr
from .sibling import thing
thing.attr
from .... import far
far.attr
";
    // Ruff 0.16.9 (TID251) for pkg/sub/mod.py with pkg/__init__.py and pkg/sub/__init__.py.
    let result = evaluate_module(&rule, "pkg/sub/mod.py", Some("pkg.sub.mod"), source);
    assert_eq!(
        found(&result, source),
        [
            "2:sibling.attr",
            "4:top.attr",
            "6:beyond.attr",
            "8:thing.attr"
        ],
        "{result}"
    );
    // Without a module path (a script) relative imports never resolve.
    let result = evaluate_module(&rule, "pkg/sub/mod.py", None, source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    // An `__init__.py` keeps `__init__` as its last segment, so `.` is its own package.
    let source = "from . import x\nx.attr\n";
    let result = evaluate_module(&rule, "pkg/__init__.py", Some("pkg.__init__"), source);
    assert_eq!(found(&result, source), ["2:x.attr"], "{result}");
}

#[test]
fn a_file_condition_reads_the_files_module_path() {
    let rule = |file: Value| python(json!({ "file": file, "rule": { "kind": "module" } }));
    let applies = |file: Value, path: &str, module: Option<&str>| {
        !starts(&evaluate_module(&rule(file), path, module, "x = 1\n")).is_empty()
    };
    // `module` reads the dotted module path: the host's, else the file's stem.
    let module = json!({ "module": { "regex": "^pkg\\.sub\\." } });
    assert!(applies(module.clone(), "pkg/sub/a.py", Some("pkg.sub.a")));
    assert!(!applies(module.clone(), "pkg/a.py", Some("pkg.a")));
    assert!(!applies(module, "pkg/sub/a.py", None));
    assert!(applies(
        json!({ "module": { "regex": "^a$" } }),
        "loose/a.py",
        None
    ));
    // A stem with a dot is the module, dots and all (Ruff's `ModuleSource::File`).
    assert!(applies(
        json!({ "module": { "regex": "^a\\.b$" } }),
        "a.b.py",
        None
    ));
    // `moduleName` reads the last segment.
    let name = json!({ "moduleName": { "regex": "^__init__$" } });
    assert!(applies(
        name.clone(),
        "pkg/__init__.py",
        Some("pkg.__init__")
    ));
    assert!(applies(name.clone(), "__init__.py", None));
    assert!(!applies(name, "pkg/a.py", Some("pkg.a")));
    // `inPackage`: whether the host sent a module path (Ruff found a package), not the stem.
    assert!(applies(
        json!({ "inPackage": true }),
        "pkg/a.py",
        Some("pkg.a")
    ));
    assert!(!applies(json!({ "inPackage": true }), "loose/a.py", None));
    assert!(applies(json!({ "inPackage": false }), "loose/a.py", None));
    assert!(applies(json!({ "inPackage": false }), "a.b.py", None));
    // All keys must hold.
    let both = json!({ "inPackage": true, "moduleName": { "regex": "^[a-z_]+$" } });
    assert!(applies(both.clone(), "pkg/a.py", Some("pkg.a")));
    assert!(!applies(both.clone(), "pkg/A.py", Some("pkg.A")));
    assert!(!applies(both, "a.py", None));
    for (language, file, expected) in [
        (
            "typescript",
            json!({ "inPackage": true }),
            "/clauses/0/forbid/ast/file/inPackage: module paths are Python's; typescript has none",
        ),
        (
            "python",
            json!([]),
            "/clauses/0/forbid/ast/file must be an object",
        ),
        (
            "python",
            json!({}),
            "/clauses/0/forbid/ast/file must set `path`, `declaration`, `module`, `moduleName`, `package`, `inPackage` or `byteOrderMark`",
        ),
        (
            "python",
            json!({ "paths": { "regex": "a" } }),
            "unknown field `paths` at /clauses/0/forbid/ast/file",
        ),
        (
            "python",
            json!({ "inPackage": "yes" }),
            "/clauses/0/forbid/ast/file/inPackage must be true or false",
        ),
        (
            "python",
            json!({ "module": "^a$" }),
            "/clauses/0/forbid/ast/file/module must be an object",
        ),
        (
            "python",
            json!({ "module": { "pattern": "a" } }),
            "unknown field `pattern` at /clauses/0/forbid/ast/file/module",
        ),
        (
            "python",
            json!({ "moduleName": { "regex": "(" } }),
            "/clauses/0/forbid/ast/file/moduleName/regex: regex parse error:\n    (\n    ^\nerror: unclosed group",
        ),
    ] {
        let ast = json!({ "file": file, "rule": { "kind": "identifier" } });
        assert_eq!(rejection(compile(language, ast.clone())), expected, "{ast}");
    }
}

#[test]
fn a_file_condition_reads_the_files_path_in_every_language() {
    let applies = |language: &str, file: Value, path: &str| {
        let (root, grammar, source) = match language {
            "python" => ("module", "python", "x = 1\n"),
            _ if path.ends_with('x') => ("program", "tsx", "let x = 1;\n"),
            _ => ("program", "typescript", "let x = 1;\n"),
        };
        let rule = compile(language, json!({ "file": file, "rule": { "kind": root } }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        !starts(&evaluate_file(&rule, grammar, path, source)).is_empty()
    };
    // `path`: a regex over the path as the host sends it, and a `notRegex`; both must hold.
    let under_src = json!({ "path": { "regex": "^src/", "notRegex": "\\.test\\.[jt]sx?$" } });
    assert!(applies("typescript", under_src.clone(), "src/a.ts"));
    assert!(applies("typescript", under_src.clone(), "src/view.tsx"));
    assert!(!applies("typescript", under_src.clone(), "src/a.test.ts"));
    assert!(!applies("typescript", under_src, "lib/a.ts"));
    assert!(applies(
        "python",
        json!({ "path": { "regex": "/tests/" } }),
        "pkg/tests/a.py"
    ));
    assert!(!applies(
        "python",
        json!({ "path": { "regex": "/tests/" } }),
        "pkg/a.py"
    ));
    // `declaration`: typescript-eslint 8.70.1's `isDefinitionFile`, over the lower-cased path.
    for (path, declaration) in [
        ("types/a.d.ts", true),
        ("a.d.mts", true),
        ("a.d.cts", true),
        ("A.D.TS", true),
        ("styles.d.css.ts", true),
        ("a.ts", false),
        ("a.d.tsx", false),
        ("d.ts", false),
        ("a.dts", false),
        ("view.tsx", false),
    ] {
        assert_eq!(
            applies("typescript", json!({ "declaration": true }), path),
            declaration,
            "{path}"
        );
        assert_eq!(
            applies("typescript", json!({ "declaration": false }), path),
            !declaration,
            "{path}"
        );
    }
    // Keys are a conjunction.
    let both = json!({ "declaration": false, "path": { "regex": "^src/" } });
    assert!(applies("typescript", both.clone(), "src/a.ts"));
    assert!(!applies("typescript", both.clone(), "src/a.d.ts"));
    assert!(!applies("typescript", both, "a.ts"));
    for (language, file, expected) in [
        (
            "typescript",
            json!({ "module": { "regex": "a" } }),
            "/clauses/0/forbid/ast/file/module: module paths are Python's; typescript has none",
        ),
        (
            "python",
            json!({ "declaration": true }),
            "/clauses/0/forbid/ast/file/declaration: declaration files are TypeScript's; python has none",
        ),
        (
            "typescript",
            json!({}),
            "/clauses/0/forbid/ast/file must set `path`, `declaration`, `package`, `jsxRuntime` or `byteOrderMark`",
        ),
        (
            "typescript",
            json!({ "declaration": "yes" }),
            "/clauses/0/forbid/ast/file/declaration must be true or false",
        ),
        (
            "typescript",
            json!({ "path": "^src/" }),
            "/clauses/0/forbid/ast/file/path must be an object",
        ),
        (
            "typescript",
            json!({ "path": {} }),
            "/clauses/0/forbid/ast/file/path must set `regex`, `notRegex` or both",
        ),
    ] {
        let ast = json!({ "file": file, "rule": { "kind": "identifier" } });
        assert_eq!(rejection(compile(language, ast.clone())), expected, "{ast}");
    }
}

#[test]
fn a_file_condition_negates_a_key_with_not_regex() {
    let rule = |file: Value| python(json!({ "file": file, "rule": { "kind": "module" } }));
    let applies = |file: Value, path: &str, module: Option<&str>| {
        !starts(&evaluate_module(&rule(file), path, module, "x = 1\n")).is_empty()
    };
    // N999's shape: an invalid name that is not an ignored one, whose parent is no migrations
    // directory. `regex` and `notRegex` on one key must both hold, and every key must hold.
    let n999 = json!({
        "inPackage": true,
        "moduleName": { "regex": "^(?:[^_a-z]|.*[^_a-z0-9])", "notRegex": "^(?:setUp|tearDown)$" },
        "module": { "notRegex": "(?:^|\\.)(?:versions|migrations)\\.[^.]*$" },
    });
    assert!(applies(n999.clone(), "pkg/Bad.py", Some("pkg.Bad")));
    assert!(!applies(n999.clone(), "pkg/setUp.py", Some("pkg.setUp")));
    assert!(!applies(n999.clone(), "pkg/good.py", Some("pkg.good")));
    assert!(!applies(
        n999.clone(),
        "pkg/migrations/Bad.py",
        Some("pkg.migrations.Bad")
    ));
    assert!(!applies(n999, "Bad.py", None));
    // `notRegex` alone holds for any module path it does not match, and never without one.
    let not_tests = json!({ "module": { "notRegex": "(?:^|\\.)tests(?:\\.|$)" } });
    assert!(applies(not_tests.clone(), "pkg/a.py", Some("pkg.a")));
    assert!(applies(not_tests.clone(), "a.py", None));
    assert!(!applies(
        not_tests.clone(),
        "pkg/tests/a.py",
        Some("pkg.tests.a")
    ));
    assert!(!applies(not_tests, "tests.py", None));
    for (file, expected) in [
        (
            json!({ "module": {} }),
            "/clauses/0/forbid/ast/file/module must set `regex`, `notRegex` or both",
        ),
        (
            json!({ "moduleName": { "notRegex": "(" } }),
            "/clauses/0/forbid/ast/file/moduleName/notRegex: regex parse error:\n    (\n    ^\nerror: unclosed group",
        ),
        (
            json!({ "module": { "notRegex": 1 } }),
            "/clauses/0/forbid/ast/file/module/notRegex must be a non-empty string",
        ),
    ] {
        let ast = json!({ "file": file, "rule": { "kind": "identifier" } });
        assert_eq!(rejection(compile("python", ast.clone())), expected, "{ast}");
    }
}

#[test]
fn dotted_directory_and_file_names_keep_their_segments() {
    // Ruff 0.16.9 (B008 with `extend-immutable-calls`, TID251 for `x.y`): `to_module_path` names each component by
    // `Path::file_stem`, so a directory `json.d` is `json`, and a dotted file stem such as `my.mod`
    // is one segment no dotted name equals. N999 and A005 read the names as on disk instead.
    let rule = python(json!({ "rule": {
        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
        "resolves": { "import": [
            "json.__init__.C", "json.d.__init__.C", "json.m.C", "json.d.m.C",
            "good.my.mod.C", "my.mod.C", "good.x.y", "good.my.x.y", "json.x.y", "json.d.x.y",
        ] },
    } }));
    let source = "class C: ...\ndef f(x=C()): ...\nfrom . import x\nx.y\n";
    let run = |path: &str, module: Option<Value>| {
        found(&evaluate_module_value(&rule, path, module, source), source)
    };
    assert_eq!(
        run("json.d/__init__.py", Some(json!(["json.d", "__init__"]))),
        ["2:C", "4:x.y"]
    );
    assert_eq!(
        run("json.d/m.py", Some(json!(["json.d", "m"]))),
        ["2:C", "4:x.y"]
    );
    // The file's own segment is already its stem; through it nothing is qualified, but a relative
    // import that drops it resolves (TID251 reports `good.x.y`).
    assert_eq!(
        run("good/my.mod.py", Some(json!(["good", "my.mod"]))),
        ["4:x.y"]
    );
    assert_eq!(run("my.mod.py", None), Vec::<String>::new());
    // The dotted string is the list of its names.
    assert_eq!(
        run("pkg/a.py", Some(json!("pkg.a"))),
        run("pkg/a.py", Some(json!(["pkg", "a"])))
    );

    let condition = |file: Value| python(json!({ "file": file, "rule": { "kind": "module" } }));
    let applies = |file: Value, path: &str, module: Option<Value>| {
        !starts(&evaluate_module_value(
            &condition(file),
            path,
            module,
            "x = 1\n",
        ))
        .is_empty()
    };
    let json_d = Some(json!(["json.d", "__init__"]));
    let my_mod = Some(json!(["good", "my.mod"]));
    // `module` and `moduleName` read the names as on disk; `package` reads the first, the top
    // package's directory (N999's `package.path().file_name()`).
    assert!(applies(
        json!({ "module": { "regex": "^json\\.d\\.__init__$" } }),
        "json.d/__init__.py",
        json_d.clone()
    ));
    assert!(applies(
        json!({ "package": { "regex": "^json\\.d$" } }),
        "json.d/__init__.py",
        json_d.clone()
    ));
    assert!(!applies(
        json!({ "package": { "regex": "^json$" } }),
        "json.d/__init__.py",
        json_d
    ));
    assert!(applies(
        json!({ "moduleName": { "regex": "^my\\.mod$" }, "inPackage": true }),
        "good/my.mod.py",
        my_mod.clone()
    ));
    assert!(applies(
        json!({ "package": { "regex": "^good$" } }),
        "good/my.mod.py",
        my_mod
    ));
    // Outside a package the stem is the module, dots and all; it has no package.
    assert!(applies(
        json!({ "moduleName": { "regex": "^my\\.mod$" }, "inPackage": false }),
        "my.mod.py",
        None
    ));
    assert!(!applies(
        json!({ "package": { "notRegex": "^$" } }),
        "my.mod.py",
        None
    ));

    for module in [
        json!([]),
        json!([""]),
        json!(["a", "b/c"]),
        json!(["a", ".."]),
        json!(["a", 1]),
    ] {
        let file =
            json!({ "language": "python", "path": "a.py", "source": "b\n", "module": module });
        let snapshot = json!({ "facts": [], "files": [file], "revision": 0 });
        let error = rule
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect_err("rejected");
        assert_eq!(
            error.reason,
            "snapshot.files/0/module must be a dotted module path (non-empty names joined by `.`) or a list of names",
            "{module}"
        );
    }
}

#[test]
fn file_conditions_match_ruffs_module_rules() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("python/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("ruff-rules.expected.json"))
            .expect("ruff-rules.expected.json"),
    )
    .expect("oracle JSON");
    let port = |file: Value| python(json!({ "file": file, "rule": { "kind": "module" } }));
    // Ruff's `is_module_name` (`[a-z_][a-z0-9_]*`, no keyword) fails; `is_migration_name` allows a
    // leading digit. Default `ignore-names` are exempt.
    let keywords = "False|None|True|and|as|assert|async|await|break|class|continue|def|del|elif|else|except|\
                    finally|for|from|global|if|import|in|is|lambda|nonlocal|not|or|pass|raise|return|try|while|with|yield";
    let invalid = format!("^(?:.*[^_a-z0-9].*|[0-9].*|{keywords})$");
    let invalid_migration = format!("^(?:.*[^_a-z0-9].*|{keywords})$");
    let ignored = "^(?:setUp|tearDown|setUpClass|tearDownClass|setUpModule|tearDownModule|asyncSetUp|asyncTearDown|\
                   setUpTestData|failureException|longMessage|maxDiff)$";
    let migration = "(?:^|\\.)(?:versions|migrations)\\.[^.]*$";
    let mut checked = 0;
    for case in expected["cases"].as_array().expect("cases") {
        let ports = match case["rule"].as_str().expect("rule") {
            "INP001" => vec![port(json!({ "inPackage": false }))],
            // The module's name, or its package's for `__init__.py`, is invalid and not ignored.
            "N999" => vec![
                port(json!({
                    "inPackage": true,
                    "moduleName": { "regex": invalid, "notRegex": format!("{ignored}|^__init__$") },
                    "module": { "notRegex": migration },
                })),
                port(json!({
                    "inPackage": true,
                    "moduleName": { "regex": invalid_migration, "notRegex": format!("{ignored}|^__init__$") },
                    "module": { "regex": migration },
                })),
                port(json!({
                    "moduleName": { "regex": "^__init__$" },
                    "package": { "regex": invalid, "notRegex": ignored },
                })),
            ],
            // A top-level module (or package) named like a standard library module.
            "A005" => vec![port(
                json!({ "module": { "regex": "^(?:logging|string|bisect|collections|xml|json)(?:\\.__init__)?$" } }),
            )],
            _ => continue,
        };
        for file in case["files"].as_array().expect("files") {
            let path = file["path"].as_str().expect("path");
            let source = file["source"].as_str().expect("source");
            let module = Some(file["module"].clone()).filter(|module| !module.is_null());
            let reported = ports.iter().any(|port| {
                !starts(&evaluate_module_value(port, path, module.clone(), source)).is_empty()
            });
            let rows = if reported { json!([1]) } else { json!([]) };
            assert_eq!(rows, file["rows"], "{} {path}", case["rule"]);
            checked += 1;
        }
    }
    assert_eq!(checked, 39);
}

#[test]
fn malformed_module_paths_are_rejected() {
    let rule =
        python(json!({ "rule": { "kind": "identifier", "resolves": { "import": ["a.b"] } } }));
    for module in [json!(""), json!("a..b"), json!(".a"), json!(3)] {
        let file =
            json!({ "language": "python", "path": "a.py", "source": "b\n", "module": module });
        let snapshot = json!({ "facts": [], "files": [file], "revision": 0 });
        let error = rule
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect_err("rejected");
        assert_eq!(
            error.reason,
            "snapshot.files/0/module must be a dotted module path (non-empty names joined by `.`) or a list of names",
            "{module}"
        );
    }
}

/// Starts of the violations of `result`.
fn starts(result: &Value) -> std::collections::BTreeSet<u64> {
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
        .collect()
}

#[test]
fn module_paths_qualify_like_ruff() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("python/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("modules.expected.json"))
            .expect("modules.expected.json"),
    )
    .expect("oracle JSON");
    let chains = python(
        json!({ "rule": { "kind": "attribute", "resolves": { "import": expected["banned"] } } }),
    );
    let callees = python(json!({ "rule": {
        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
        "resolves": { "import": expected["immutableCalls"] },
    } }));
    let files = expected["files"].as_array().expect("files");
    assert_eq!(files.len(), 5);
    for file in files {
        let path = file["path"].as_str().expect("path");
        let module = file["module"].as_str();
        let source = std::fs::read_to_string(root.join("modules").join(path)).expect("fixture");
        let banned = file["banned"]
            .as_array()
            .expect("banned")
            .iter()
            .map(|item| item["start"].as_u64().expect("start"))
            .collect::<std::collections::BTreeSet<_>>();
        let result = evaluate_module(&chains, path, module, &source);
        assert_eq!(starts(&result), banned, "{path} (TID251)");
        let offsets = |key: &str| {
            file[key]
                .as_array()
                .expect(key)
                .iter()
                .map(|start| start.as_u64().expect("start"))
                .collect::<std::collections::BTreeSet<_>>()
        };
        let (immutable, mutable) = (offsets("immutable"), offsets("mutable"));
        let resolved = starts(&evaluate_module(&callees, path, module, &source));
        let at_calls = resolved
            .into_iter()
            .filter(|start| immutable.contains(start) || mutable.contains(start))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(at_calls, immutable, "{path} (B008)");
    }
}

#[test]
fn language_options_declare_the_globals_eslint_declares() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("ecma/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("eslint.expected.json")).expect("eslint.expected.json"),
    )
    .expect("oracle JSON");
    let files = expected["files"].as_array().expect("files");
    assert_eq!(files.len(), 4);
    let word = regex::Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*").expect("regex");
    let mut cases = 0;
    for file in files {
        let path = file["path"].as_str().expect("path");
        let source = std::fs::read_to_string(root.join("eslint").join(path)).expect("fixture");
        // Every word of the file is listed, so the options alone decide which references match.
        let names = word
            .find_iter(&source)
            .map(|word| word.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        for case in file["cases"].as_array().expect("cases") {
            let rule = typescript(json!({
                "languageOptions": case["languageOptions"],
                "rule": { "resolves": { "global": names, "directives": "eslint" } },
            }));
            let globals = case["globals"]
                .as_array()
                .expect("globals")
                .iter()
                .map(|global| global["start"].as_u64().expect("start"))
                .collect::<std::collections::BTreeSet<_>>();
            let result = evaluate_file(&rule, "typescript", "fixture.js", &source);
            assert_eq!(
                starts(&result),
                globals,
                "{path} {}",
                case["languageOptions"]
            );
            cases += 1;
        }
    }
    assert_eq!(cases, 20);
}

#[test]
fn port_shaped_rules_report_what_eslint_reports_under_language_options() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("ecma/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("eslint-rules.expected.json"))
            .expect("eslint-rules.expected.json"),
    )
    .expect("oracle JSON");
    let calls = |names: &[&str], forms: &[&str]| {
        json!({
            "rule": { "any": forms.iter().map(|form| json!({ "pattern": form })).collect::<Vec<_>>() },
            "constraints": { "F": { "resolves": { "global": names, "directives": "eslint" } } },
        })
    };
    let mut checked = 0;
    for case in expected["cases"].as_array().expect("cases") {
        let directive = json!({ "kind": "expression_statement", "regex": "^'use strict';$" });
        // strict ("safe"): one clause per mode, each gated on the configuration that selects it.
        let script = json!({ "sourceType": "script", "ecmaFeatures": { "globalReturn": false, "impliedStrict": false } });
        let global_modes = [
            json!({ "sourceType": "commonjs", "ecmaFeatures": { "impliedStrict": false } }),
            json!({ "sourceType": "script", "ecmaFeatures": { "globalReturn": true, "impliedStrict": false } }),
        ];
        let asts = match case["rule"].as_str().expect("rule") {
            "strict" => {
                let mut asts = vec![
                    json!({ "when": { "sourceType": "module" }, "rule": directive }),
                    json!({ "when": { "ecmaFeatures": { "impliedStrict": true } }, "rule": directive }),
                    json!({ "when": script, "rule": { "any": [
                        { "all": [directive, { "inside": { "kind": "program" } }] },
                        { "kind": "function_declaration", "all": [
                            { "not": { "inside": { "kind": "function_declaration", "stopBy": "end" } } },
                            { "not": { "has": { "kind": "statement_block", "has": directive } } },
                        ] },
                    ] } }),
                ];
                for when in global_modes {
                    asts.push(json!({ "when": when, "rule": { "all": [directive, { "inside": { "kind": "statement_block" } }] } }));
                    asts.push(json!({ "when": when, "rule": { "kind": "program", "not": { "has": directive } } }));
                }
                asts
            }
            rule => vec![match rule {
                // The global itself, or a variable whose binding carries it (a recursive util).
                "no-obj-calls" => json!({
                    "rule": { "any": [{ "pattern": "$F($$$)" }, { "pattern": "new $F($$$)" }] },
                    "constraints": { "F": { "matches": "object" } },
                    "utils": { "object": { "kind": "identifier", "any": [
                        { "resolves": { "global": ["Math", "JSON", "Reflect", "Atomics", "Intl"], "directives": "eslint" } },
                        { "resolves": { "binding": { "value": { "matches": "object" } } } },
                    ] } },
                }),
                // A string first argument: a literal, a template, a `+` with one, or a value
                // getStaticValue reads through the call's scope.
                "no-implied-eval" => json!({
                    "rule": { "any": [{ "pattern": "$F($S)" }, { "pattern": "$F($S, $$$)" }] },
                    "constraints": {
                        "F": { "resolves": { "global": ["setTimeout", "setInterval", "execScript"] } },
                        "S": { "any": [{ "matches": "evaluated" }, { "value": { "type": "string", "follow": true } }] },
                    },
                    "utils": { "evaluated": { "any": [
                        { "kind": "string" },
                        { "kind": "template_string" },
                        { "kind": "binary_expression", "has": { "regex": "^\\+$" },
                          "all": [{ "has": { "matches": "evaluated" } }] },
                    ] } },
                }),
                "no-new-wrappers" => calls(&["String", "Number", "Boolean"], &["new $F($$$)"]),
                "no-eval" => json!({ "rule": {
                "kind": "property_identifier",
                "regex": "^eval$",
                "inside": { "kind": "member_expression", "has": { "any": [
                    { "kind": "this", "resolves": { "this": "global", "sloppyFunctions": true } },
                    { "kind": "identifier", "resolves": { "global": ["window", "global", "globalThis"], "directives": "eslint", "scriptDeclarations": true } },
                ] } },
            } }),
                "no-extend-native" => json!({ "rule": {
                "kind": "assignment_expression",
                "has": { "kind": "member_expression", "has": { "kind": "member_expression", "all": [
                    { "has": { "kind": "property_identifier", "regex": "^prototype$" } },
                    { "has": { "kind": "identifier", "resolves": { "global": ["Array", "Object", "Map", "Set", "Promise"], "directives": "eslint", "scriptDeclarations": true } } },
                ] } },
            } }),
                // One clause: `declared: false` asks only whether a name is shadowed.
                "no-alert" => json!({ "rule": {
                "kind": "call_expression",
                "has": { "any": [
                    { "kind": "identifier", "resolves": { "global": ["alert", "confirm", "prompt"], "declared": false } },
                    { "kind": "member_expression", "all": [
                        { "has": { "kind": "property_identifier", "regex": "^(alert|confirm|prompt)$" } },
                        { "has": { "any": [
                            { "kind": "identifier", "resolves": { "global": ["window"], "declared": false } },
                            { "kind": "this", "resolves": { "this": "globalScope" } },
                        ] } },
                    ] },
                ] },
            } }),
                other => panic!("no port for {other}"),
            }],
        };
        let source = case["code"].as_str().expect("code");
        let reported = case["reported"]
            .as_array()
            .expect("reported")
            .iter()
            .map(|start| start.as_u64().expect("start"))
            .collect::<std::collections::BTreeSet<_>>();
        let mut found = std::collections::BTreeSet::new();
        for mut ast in asts {
            ast["languageOptions"] = case["languageOptions"].clone();
            found.extend(starts(&evaluate_file(
                &typescript(ast),
                "typescript",
                "fixture.js",
                source,
            )));
        }
        assert_eq!(
            found, reported,
            "{} {}",
            case["rule"], case["languageOptions"]
        );
        checked += 1;
    }
    assert_eq!(checked, 58);
}

#[test]
fn a_typing_modules_member_is_typing_s_own_for_import_and_imported() {
    let source = "\
from std import overload
import std.overload
from other import overload as o
import std
std.overload
std.cast
other.overload
overload
o
";
    let rules = |options: Option<Value>| {
        let mut ast = json!({ "rule": {
            "any": [{ "kind": "dotted_name" }, { "kind": "identifier" }, { "kind": "attribute" }],
            "not": { "inside": { "kind": "attribute" } },
            "resolves": { "imported": ["typing.overload"], "import": ["typing.overload"] },
        } });
        if let Some(options) = options {
            ast["languageOptions"] = options;
        }
        python(ast)
    };
    let plain = rules(None);
    let result = evaluate_file(&plain, "python", "a.py", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    // Only `typing.<member>` is widened, to the member of a listed module and nothing under it.
    let std = rules(Some(json!({ "typingModules": ["std"] })));
    let result = evaluate_file(&std, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "1:overload",
            "2:std.overload",
            "5:std.overload",
            "8:overload"
        ],
        "{result}"
    );
}

#[test]
fn imported_matches_the_names_an_import_statement_imports() {
    let source = "\
import pickle
import xml.etree.cElementTree as ET
import os, subprocess
from pickle import loads, dumps as d
from xml.etree import ElementTree
from . import pickle
from .pickle import x
from pickle import *
from __future__ import annotations
pickle.loads(a)
";
    let exact = python(json!({ "rule": {
        "kind": "dotted_name",
        "resolves": { "imported": ["pickle", "xml.etree.cElementTree", "pickle.loads", "__future__.annotations"] },
    } }));
    let result = evaluate_file(&exact, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "1:pickle",
            "2:xml.etree.cElementTree",
            "4:pickle",
            "4:loads",
            "8:pickle",
            "9:annotations"
        ],
        "{result}"
    );
    // A prefix matches the name itself and every name under it.
    let prefix = python(json!({ "rule": {
        "kind": "dotted_name",
        "resolves": { "imported": ["pickle", "xml.etree"], "prefix": true },
    } }));
    let result = evaluate_file(&prefix, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "1:pickle",
            "2:xml.etree.cElementTree",
            "4:pickle",
            "4:loads",
            "4:dumps",
            "5:xml.etree",
            "5:ElementTree",
            "8:pickle"
        ],
        "{result}"
    );
    // `xml.etreex` is no name under `xml.etree`.
    let result = evaluate_file(
        &prefix,
        "python",
        "a.py",
        "import xml.etreex\nimport pickles\n",
    );
    assert_eq!(
        found(&result, "import xml.etreex\nimport pickles\n"),
        Vec::<String>::new(),
        "{result}"
    );
}

#[test]
fn relative_text_names_relative_imports_by_their_module_text() {
    let rule = |relative: Option<&str>| {
        let mut resolves = json!({ "imported": ["pdb", "pdb.set_trace", "x.pdb", "ipdb"] });
        if let Some(relative) = relative {
            resolves["relative"] = json!(relative);
        }
        python(json!({ "rule": { "kind": "dotted_name", "resolves": resolves } }))
    };
    let source = "\
from .pdb import set_trace
from . import pdb
from .. import (ipdb as i)
from .x import pdb
from pdb import set_trace
";
    let run = |relative: Option<&str>| {
        let result = evaluate_file(&rule(relative), "python", "a.py", source);
        found(&result, source)
    };
    let absolute = ["5:pdb", "5:set_trace"];
    assert_eq!(run(None), absolute);
    assert_eq!(run(Some("never")), absolute);
    // The dots are ignored: `.pdb` is the module `pdb`, `from . import pdb` imports `pdb`.
    assert_eq!(
        run(Some("text")),
        [
            "1:pdb",
            "1:set_trace",
            "2:pdb",
            "3:ipdb",
            "4:pdb",
            "5:pdb",
            "5:set_trace"
        ]
    );
}

#[test]
fn relative_text_imports_match_ruffs_t100() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("python/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("ruff-rules.expected.json"))
            .expect("ruff-rules.expected.json"),
    )
    .expect("oracle JSON");
    // T100's import checks: a debugger call imported by name, or a debugger module imported
    // whole (`import pdb`, `from . import pdb`), not a module a `from` statement names.
    let calls = json!([
        "pdb.set_trace",
        "pudb.set_trace",
        "ipdb.set_trace",
        "ipdb.sset_trace",
        "IPython.terminal.embed.InteractiveShellEmbed",
        "celery.contrib.rdb.set_trace",
        "builtins.breakpoint",
        "sys.breakpointhook",
        "sys.__breakpointhook__"
    ]);
    let modules = json!([
        "pdb",
        "pudb",
        "ipdb",
        "debugpy",
        "ptvsd",
        "IPython.terminal.embed",
        "celery.contrib.rdb"
    ]);
    let port = python(json!({ "rule": { "kind": "dotted_name", "any": [
        { "resolves": { "imported": calls, "relative": "text" } },
        { "resolves": { "imported": modules, "relative": "text" }, "any": [
            { "inside": { "kind": "import_statement" } },
            { "inside": { "kind": "aliased_import", "inside": { "kind": "import_statement" } } },
            { "inside": { "kind": "import_from_statement", "has": { "kind": "relative_import", "not": { "has": { "kind": "dotted_name" } } } } },
            { "inside": { "kind": "aliased_import", "inside": { "kind": "import_from_statement", "has": { "kind": "relative_import", "not": { "has": { "kind": "dotted_name" } } } } } },
        ] },
    ] } }));
    let mut checked = 0;
    for case in expected["cases"].as_array().expect("cases") {
        if case["rule"] != "T100" {
            continue;
        }
        for file in case["files"].as_array().expect("files") {
            let path = file["path"].as_str().expect("path");
            let source = file["source"].as_str().expect("source");
            let result = evaluate_module(&port, path, file["module"].as_str(), source);
            let lines = found(&result, source)
                .iter()
                .map(|found| {
                    found
                        .split(':')
                        .next()
                        .expect("line")
                        .parse::<u64>()
                        .expect("line")
                })
                .collect::<std::collections::BTreeSet<_>>();
            let rows = file["rows"]
                .as_array()
                .expect("rows")
                .iter()
                .map(|row| row.as_u64().expect("row"))
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(lines, rows, "{path}");
            checked += 1;
        }
    }
    assert_eq!(checked, 3);
}

#[test]
fn prefix_extends_import_to_every_name_under_a_listed_one() {
    let rule = python(json!({ "rule": {
        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
        "resolves": { "import": ["subprocess"], "prefix": true },
    } }));
    let source =
        "import subprocess as sp\nsp.run(a)\nsp.Popen.wait\nimport subprocessx\nsubprocessx.run\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["2:sp", "2:sp.run", "3:sp.Popen", "3:sp", "3:sp.Popen.wait"],
        "{result}"
    );
}

#[test]
fn malformed_imported_and_prefix_are_rejected() {
    for (language, ast, expected) in [
        (
            "python",
            json!({ "rule": { "kind": "dotted_name", "resolves": { "imported": [] } } }),
            "/clauses/0/forbid/ast/rule/resolves/imported must not be empty",
        ),
        (
            "python",
            json!({ "rule": { "kind": "dotted_name", "resolves": { "imported": ["a..b"] } } }),
            "/clauses/0/forbid/ast/rule/resolves/imported/0: `a..b` is not a dotted name (identifiers joined by `.`), a Rust path (identifiers joined by `::`) or a PHP name (labels joined by `\\`) of at most 256 bytes",
        ),
        (
            "python",
            json!({ "rule": { "kind": "dotted_name", "resolves": { "imported": ["a"], "prefix": "yes" } } }),
            "/clauses/0/forbid/ast/rule/resolves/prefix must be true",
        ),
        (
            "python",
            json!({ "rule": { "kind": "identifier", "resolves": { "global": ["a"], "prefix": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/prefix: only `import` and `imported` names are dotted; list some",
        ),
        (
            "typescript",
            json!({ "rule": { "resolves": { "imported": ["react"] } } }),
            "/clauses/0/forbid/ast/rule/resolves/imported: import resolution is Python-only for now; TypeScript rules can use `global`",
        ),
        (
            "python",
            json!({ "rule": { "kind": "dotted_name", "resolves": { "imported": ["a"], "relative": "level" } } }),
            "/clauses/0/forbid/ast/rule/resolves/relative must be \"never\" or \"text\"",
        ),
        (
            "python",
            json!({ "rule": { "kind": "attribute", "resolves": { "import": ["a"], "relative": "text" } } }),
            "/clauses/0/forbid/ast/rule/resolves/relative: only `imported` names import sites; list some",
        ),
    ] {
        assert_eq!(rejection(compile(language, ast.clone())), expected, "{ast}");
    }
}

#[test]
fn python_version_limits_global_to_its_builtins() {
    let names = [
        "print",
        "aiter",
        "ExceptionGroup",
        "frozendict",
        "__annotate__",
        "foo",
    ];
    let source = "print; aiter; ExceptionGroup; frozendict; __annotate__; foo\n";
    let run = |options: Option<Value>| {
        let mut ast = json!({ "rule": { "kind": "identifier", "resolves": { "global": names } } });
        if let Some(options) = options {
            ast["languageOptions"] = options;
        }
        found(
            &evaluate_file(&python(ast), "python", "a.py", source),
            source,
        )
    };
    // Without options every listed, unbound name matches (M7).
    assert_eq!(run(None).len(), 6);
    // Ruff 0.16.9's builtins per --target-version; Ruff's default target is 3.10.
    assert_eq!(run(Some(json!({}))), ["1:print", "1:aiter"]);
    assert_eq!(run(Some(json!({ "pythonVersion": "3.9" }))), ["1:print"]);
    assert_eq!(
        run(Some(json!({ "pythonVersion": "3.11" }))),
        ["1:print", "1:aiter", "1:ExceptionGroup"]
    );
    assert_eq!(
        run(Some(json!({ "pythonVersion": "3.14" }))),
        ["1:print", "1:aiter", "1:ExceptionGroup", "1:__annotate__"]
    );
    assert_eq!(
        run(Some(json!({ "pythonVersion": "3.15" }))),
        [
            "1:print",
            "1:aiter",
            "1:ExceptionGroup",
            "1:frozendict",
            "1:__annotate__"
        ]
    );
}

#[test]
fn python_three_fourteen_reads_annotations_after_the_module() {
    // Ruff 0.16.9: at py313 every annotation here is read before `Sequence` is imported; at py314
    // (PEP 649) they are deferred, as under `from __future__ import annotations`.
    let source = "\
def f(x: Sequence) -> Sequence:
    pass
y: Sequence = 1
from typing import Sequence
";
    let rule = |version: &str| {
        python(json!({
            "languageOptions": { "pythonVersion": version },
            "rule": { "kind": "identifier", "resolves": { "import": ["typing.Sequence"] } },
        }))
    };
    let result = evaluate_file(&rule("3.13"), "python", "a.py", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    let result = evaluate_file(&rule("3.14"), "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["1:Sequence", "1:Sequence", "3:Sequence"],
        "{result}"
    );
}

#[test]
fn a_version_gate_makes_a_query_apply_only_to_its_versions() {
    let gated = |language: &str, options: Value, when: Value| {
        compile(
            language,
            json!({ "languageOptions": options, "when": when, "rule": { "kind": "identifier" } }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason))
    };
    let python_source = "zip\n";
    let python = |version: Option<&str>, when: Value| {
        let options = version.map_or(json!({}), |version| json!({ "pythonVersion": version }));
        let result = evaluate_file(
            &gated("python", options, when),
            "python",
            "a.py",
            python_source,
        );
        // Not applicable is sound: no violation and no failure.
        found(&result, python_source).len()
    };
    let from_310 = json!({ "pythonVersion": { "min": "3.10" } });
    assert_eq!(python(Some("3.9"), from_310.clone()), 0);
    assert_eq!(python(Some("3.10"), from_310.clone()), 1);
    assert_eq!(python(Some("3.14"), from_310.clone()), 1);
    // Without pythonVersion the gate reads Ruff's default target, 3.10.
    assert_eq!(python(None, from_310), 1);
    let window = json!({ "pythonVersion": { "min": "3.8", "max": "3.11" } });
    assert_eq!(python(Some("3.7"), window.clone()), 0);
    assert_eq!(python(Some("3.8"), window.clone()), 1);
    assert_eq!(python(Some("3.11"), window.clone()), 1);
    assert_eq!(python(Some("3.12"), window), 0);
    let typescript_source = "a;\n";
    let typescript = |options: Value, when: Value| {
        let result = evaluate_file(
            &gated("typescript", options, when),
            "typescript",
            "a.ts",
            typescript_source,
        );
        found(&result, typescript_source).len()
    };
    let es2015_to_2020 = json!({ "ecmaVersion": { "min": 6, "max": 2020 } });
    assert_eq!(
        typescript(
            json!({ "ecmaVersion": 5, "sourceType": "script" }),
            es2015_to_2020.clone()
        ),
        0
    );
    assert_eq!(
        typescript(json!({ "ecmaVersion": 2015 }), es2015_to_2020.clone()),
        1
    );
    assert_eq!(
        typescript(json!({ "ecmaVersion": 11 }), es2015_to_2020.clone()),
        1
    );
    // Without ecmaVersion the gate reads ESLint's default, the latest.
    assert_eq!(typescript(json!({}), es2015_to_2020), 0);
    assert_eq!(
        typescript(json!({}), json!({ "ecmaVersion": { "min": "latest" } })),
        1
    );
}

#[test]
fn a_source_gate_makes_a_query_apply_only_to_its_source_types_and_features() {
    let source = "a;\n";
    let applies = |options: Option<Value>, when: Value| {
        let mut ast = json!({ "when": when, "rule": { "kind": "identifier" } });
        if let Some(options) = options {
            ast["languageOptions"] = options;
        }
        let result = evaluate_file(&typescript(ast), "typescript", "a.js", source);
        // Not applicable is sound: no violation and no failure.
        !found(&result, source).is_empty()
    };
    let script = Some(json!({ "sourceType": "script" }));
    let commonjs = Some(json!({ "sourceType": "commonjs" }));
    let implied = Some(
        json!({ "sourceType": "script", "parserOptions": { "ecmaFeatures": { "impliedStrict": true } } }),
    );
    let global_return = Some(
        json!({ "sourceType": "script", "parserOptions": { "ecmaFeatures": { "globalReturn": true } } }),
    );
    // One source type or a list of them; without languageOptions ESLint's default, a module.
    assert!(applies(script.clone(), json!({ "sourceType": "script" })));
    assert!(!applies(
        commonjs.clone(),
        json!({ "sourceType": "script" })
    ));
    assert!(applies(
        commonjs.clone(),
        json!({ "sourceType": ["script", "commonjs"] })
    ));
    assert!(applies(None, json!({ "sourceType": "module" })));
    assert!(applies(
        Some(json!({})),
        json!({ "sourceType": ["module"] })
    ));
    assert!(!applies(
        None,
        json!({ "sourceType": ["script", "commonjs"] })
    ));
    // `ecmaFeatures` flags must equal the query's (default false).
    let strict = json!({ "ecmaFeatures": { "impliedStrict": true } });
    assert!(applies(implied.clone(), strict.clone()));
    assert!(!applies(script.clone(), strict));
    let sloppy_script = json!({ "sourceType": "script", "ecmaFeatures": { "impliedStrict": false, "globalReturn": false } });
    assert!(applies(script.clone(), sloppy_script.clone()));
    assert!(!applies(implied.clone(), sloppy_script.clone()));
    assert!(!applies(global_return.clone(), sloppy_script));
    assert!(applies(
        global_return,
        json!({ "ecmaFeatures": { "globalReturn": true } })
    ));
    assert!(!applies(
        None,
        json!({ "ecmaFeatures": { "globalReturn": true } })
    ));
    // Every key must hold, the version range too.
    let both = json!({ "sourceType": "script", "ecmaVersion": { "max": 5 } });
    assert!(applies(
        Some(json!({ "sourceType": "script", "ecmaVersion": 5 })),
        both.clone()
    ));
    assert!(!applies(script, both.clone()));
    assert!(!applies(
        Some(json!({ "sourceType": "commonjs", "ecmaVersion": 5 })),
        both
    ));

    for (language, when, expected) in [
        (
            "typescript",
            json!({ "sourceType": "esm" }),
            "/clauses/0/forbid/ast/when/sourceType must be \"script\", \"module\" or \"commonjs\", or a non-empty list of them",
        ),
        (
            "typescript",
            json!({ "sourceType": [] }),
            "/clauses/0/forbid/ast/when/sourceType must be \"script\", \"module\" or \"commonjs\", or a non-empty list of them",
        ),
        (
            "typescript",
            json!({ "ecmaFeatures": {} }),
            "/clauses/0/forbid/ast/when/ecmaFeatures must set `globalReturn`, `impliedStrict` or both",
        ),
        (
            "typescript",
            json!({ "ecmaFeatures": { "jsx": true } }),
            "unknown field `jsx` at /clauses/0/forbid/ast/when/ecmaFeatures",
        ),
        (
            "typescript",
            json!({ "ecmaFeatures": { "impliedStrict": "yes" } }),
            "/clauses/0/forbid/ast/when/ecmaFeatures/impliedStrict must be true or false",
        ),
        (
            "python",
            json!({ "sourceType": "script" }),
            "unknown field `sourceType` at /clauses/0/forbid/ast/when",
        ),
    ] {
        let ast = json!({ "when": when, "rule": { "kind": "identifier" } });
        assert_eq!(rejection(compile(language, ast.clone())), expected, "{ast}");
    }
}

#[test]
fn a_source_gate_without_a_source_type_reads_each_files_own() {
    let source = "a;\n";
    let applies = |options: Option<Value>, path: &str, package: Option<&str>, source: &str| {
        let mut ast =
            json!({ "when": { "sourceType": "commonjs" }, "rule": { "kind": "identifier" } });
        if let Some(options) = options {
            ast["languageOptions"] = options;
        }
        !found(
            &evaluate_in_package(&typescript(ast), path, package, source),
            source,
        )
        .is_empty()
    };
    for options in [
        None,
        Some(json!({})),
        Some(json!({ "globals": { "a": "readonly" } })),
    ] {
        for (path, package, expected) in [
            ("a.cjs", None, true),
            ("a.cts", None, true),
            ("a.js", Some("commonjs"), true),
            ("a.js", Some("unset"), true),
            ("a.mjs", None, false),
            ("a.ts", None, false),
            ("a.ts", Some("commonjs"), true),
            ("a.ts", Some("unset"), true),
            ("a.tsx", Some("commonjs"), false),
            ("a.d.ts", Some("commonjs"), false),
            ("a.js", None, false),
            ("a.js", Some("module"), false),
        ] {
            assert_eq!(
                applies(options.clone(), path, package, source),
                expected,
                "{options:?} {path} {package:?}"
            );
        }
        // The syntax decides an unset package's `.js` file once it is read: a declaration, a
        // top-level `await` (not one inside a function) or a lexical redeclaration of a CommonJS
        // wrapper name (not a `var` one) makes a module.
        for (syntax, commonjs) in [
            ("export {};\n", false),
            ("await Promise.resolve();\n", false),
            ("if (a) { for await (const b of c) {} }\n", false),
            ("const require = createRequire(import.meta.url);\n", false),
            ("let { module } = b;\n", false),
            ("class exports {}\n", false),
            ("async function f() { await b; }\n", true),
            ("const f = async () => { await b; };\n", true),
            ("var require = b;\n", true),
            ("const other = 1;\n", true),
            ("const { b = require } = c;\n", true),
            // A class member's computed name runs where the class is defined; its body and a
            // field initializer do not.
            ("class C { [await b]() {} }\n", false),
            ("class C { static [await b] = 1; }\n", false),
            ("({ [await b]() {} });\n", false),
            ("class C { async m() { await b; } }\n", true),
            ("class C { x = async () => { await b; }; }\n", true),
        ] {
            let source = format!("{syntax}a;\n");
            assert_eq!(
                applies(options.clone(), "a.js", Some("unset"), &source),
                commonjs,
                "{options:?} {syntax}"
            );
        }
        // In a `.ts` file, after type stripping: an abstract class is a class, while `declare`
        // declarations and type-only imports are erased.
        for (syntax, commonjs) in [
            ("abstract class exports {}\n", false),
            ("export abstract class B {}\n", false),
            ("declare class exports {}\n", true),
            ("declare const require: unknown;\n", true),
            ("import type { B } from 'b';\n", true),
        ] {
            let source = format!("{syntax}a;\n");
            assert_eq!(
                applies(options.clone(), "a.ts", Some("unset"), &source),
                commonjs,
                "{options:?} {syntax}"
            );
        }
    }
    // A pinned `sourceType` decides for every file.
    let pinned = Some(json!({ "sourceType": "commonjs" }));
    assert!(applies(pinned.clone(), "a.mjs", None, source));
    assert!(applies(pinned, "a.js", Some("module"), source));
    assert!(!applies(
        Some(json!({ "sourceType": "module" })),
        "a.cjs",
        None,
        source
    ));
}

/// Node's syntax detection reads an untyped package's file however deeply its top level nests:
/// a binary chain far deeper than the native stack allows recursion over, with `await` and a
/// destructured wrapper name only in its text, still decides the module system.
#[test]
fn syntax_detection_reads_a_deeply_nested_top_level() {
    let rule =
        typescript(json!({ "when": { "sourceType": "commonjs" }, "rule": { "kind": "program" } }));
    let deep = format!("// await\nx = {}a;\n", "a + ".repeat(200_000));
    assert_eq!(
        found(
            &evaluate_in_package(&rule, "a.js", Some("unset"), &deep),
            &deep
        )
        .len(),
        1,
        "no top-level await: CommonJS"
    );
    let nested = format!(
        "const {}require{} = x;\n",
        "{ a: [".repeat(50_000),
        "] }".repeat(50_000)
    );
    assert!(
        found(
            &evaluate_in_package(&rule, "a.js", Some("unset"), &nested),
            &nested
        )
        .is_empty(),
        "a destructured wrapper name: a module"
    );
}

#[test]
fn a_gate_that_does_not_hold_keeps_the_querys_captures() {
    // Captures, `constraints`, `compare`, capture anchors and message and fix templates are
    // checked against the whole query whether its gate holds or not; it just matches no file.
    let gated = |language: &str, options: Option<Value>, when: Value| {
        let (kind, source, path) = match language {
            "python" => ("function_definition", "def foo():\n    pass\n", "a.py"),
            _ => ("function_declaration", "function foo() {}\n", "a.js"),
        };
        let mut ast = json!({
            "language": language,
            "when": when,
            "rule": {
                "kind": kind,
                "has": { "stopBy": "neighbor", "kind": "identifier", "pattern": "$NAME" },
                "compare": { "left": "NAME", "op": "eq", "right": "NAME" },
            },
            "constraints": { "NAME": { "regex": "^f" } },
        });
        if let Some(options) = options {
            ast["languageOptions"] = options;
        }
        let clause = json!({
            "id": "ast-match",
            "forbid": { "ast": ast },
            "violation": { "anchor": "capture.NAME", "code": "X", "message": "fn $NAME", "fix": "$NAME" },
        });
        let compiled = compile_document_with(
            clause_document(clause.clone()),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{clause}: {}", error.reason));
        let result = evaluate_file(&compiled, language, path, source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message").to_owned())
            .collect::<Vec<_>>()
    };
    let none: [&str; 0] = [];
    let python_310 = Some(json!({ "pythonVersion": "3.10" }));
    let python_39 = Some(json!({ "pythonVersion": "3.9" }));
    let from_310 = json!({ "pythonVersion": { "min": "3.10" } });
    assert_eq!(gated("python", python_310, from_310.clone()), ["fn foo"]);
    assert_eq!(gated("python", python_39, from_310), none);
    let es3 = json!({ "ecmaVersion": { "max": 3 } });
    assert_eq!(
        gated(
            "typescript",
            Some(json!({ "ecmaVersion": 3, "sourceType": "script" })),
            es3.clone()
        ),
        ["fn foo"]
    );
    assert_eq!(gated("typescript", None, es3), none);
    let script = json!({ "sourceType": "script" });
    let script_options = Some(json!({ "sourceType": "script" }));
    assert_eq!(
        gated("typescript", script_options, script.clone()),
        ["fn foo"]
    );
    assert_eq!(gated("typescript", Some(json!({})), script.clone()), none);
    assert_eq!(gated("typescript", None, script), none);
    let implied = json!({ "ecmaFeatures": { "impliedStrict": true } });
    assert_eq!(
        gated(
            "typescript",
            Some(json!({ "parserOptions": { "ecmaFeatures": { "impliedStrict": true } } })),
            implied.clone()
        ),
        ["fn foo"]
    );
    assert_eq!(gated("typescript", Some(json!({})), implied), none);
}

#[test]
fn version_gates_match_ruffs_b905() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("python/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("ruff-rules.expected.json"))
            .expect("ruff-rules.expected.json"),
    )
    .expect("oracle JSON");
    let mut checked = 0;
    for case in expected["cases"].as_array().expect("cases") {
        if case["rule"] != "B905" {
            continue;
        }
        let target = case["target"].as_str().expect("target");
        let version = format!("3.{}", target.strip_prefix("py3").expect("py3N"));
        let port = python(json!({
            "languageOptions": { "pythonVersion": version },
            "when": { "pythonVersion": { "min": "3.10" } },
            "rule": { "kind": "call", "all": [
                { "has": { "kind": "identifier", "regex": "^zip$", "resolves": { "global": ["zip"] } } },
                { "not": { "has": { "kind": "argument_list", "has": { "kind": "keyword_argument", "has": { "kind": "identifier", "regex": "^strict$" } } } } },
            ] },
        }));
        for file in case["files"].as_array().expect("files") {
            let source = file["source"].as_str().expect("source");
            let result = evaluate_file(&port, "python", "b905.py", source);
            let lines = found(&result, source)
                .iter()
                .map(|found| {
                    found
                        .split(':')
                        .next()
                        .expect("line")
                        .parse::<u64>()
                        .expect("line")
                })
                .collect::<Vec<_>>();
            assert_eq!(json!(lines), file["rows"], "{target}");
            checked += 1;
        }
    }
    assert_eq!(checked, 3);
}

#[test]
fn seen_before_matches_ruffs_seen_module() {
    let root = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("python/scope");
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("ruff-rules.expected.json"))
            .expect("ruff-rules.expected.json"),
    )
    .expect("oracle JSON");
    // SIM112 reads `os.environ[...]` by its text once `os` is seen; ASYNC109 at 3.10 needs anyio
    // or trio seen where the `def` is.
    let sim112 = python(json!({ "rule": { "kind": "subscript", "all": [
        { "pattern": "os.environ[$K]" },
        { "resolves": { "imported": ["os"], "seenBefore": true } },
    ] } }));
    let async109 = python(json!({ "rule": {
        "kind": "identifier", "regex": "^timeout$",
        "inside": { "kind": "parameters", "inside": { "kind": "function_definition", "regex": "^async" } },
        "resolves": { "imported": ["anyio", "trio"], "seenBefore": true },
    } }));
    let mut checked = 0;
    for case in expected["cases"].as_array().expect("cases") {
        let port = match case["rule"].as_str().expect("rule") {
            "SIM112" => &sim112,
            "ASYNC109" => &async109,
            _ => continue,
        };
        for file in case["files"].as_array().expect("files") {
            let path = file["path"].as_str().expect("path");
            let source = file["source"].as_str().expect("source");
            let result = evaluate_file(port, "python", path, source);
            let lines = found(&result, source)
                .iter()
                .map(|found| {
                    found
                        .split(':')
                        .next()
                        .expect("line")
                        .parse::<u64>()
                        .expect("line")
                })
                .collect::<Vec<_>>();
            assert_eq!(json!(lines), file["rows"], "{} {path}", case["rule"]);
            checked += 1;
        }
    }
    assert_eq!(checked, 4);
}

/// `seenBefore` under M10b's `file` conditions: the condition picks the grammar by the path, and
/// the traversal order is the same in a `.pyi` stub, whose table reads names after the module.
#[test]
fn seen_before_holds_under_file_conditions_and_in_stubs() {
    let source = "\
os.environ[\"A\"]
def f():
    os.environ[\"B\"]
import os
os.environ[\"C\"]
";
    let sim112 = |file: Option<Value>| {
        let mut ast = json!({ "rule": { "kind": "subscript", "all": [
            { "pattern": "os.environ[$K]" },
            { "resolves": { "imported": ["os"], "seenBefore": true } },
        ] } });
        if let Some(file) = file {
            ast["file"] = file;
        }
        python(ast)
    };
    let lines = |compiled: &CompiledCodeRuleDsl, path: &str| {
        found(&evaluate_file(compiled, "python", path, source), source)
            .iter()
            .map(|found| found.split(':').next().expect("line").to_owned())
            .collect::<Vec<_>>()
    };
    // The module body first, then `f`'s: only the read before the import is unseen.
    for path in ["a.py", "pkg/a.py", "a.pyi", "pkg/a.pyi"] {
        assert_eq!(lines(&sim112(None), path), ["3", "5"], "{path}");
    }
    let packaged = sim112(Some(json!({ "path": { "regex": "^pkg/" } })));
    assert_eq!(lines(&packaged, "pkg/a.py"), ["3", "5"]);
    assert_eq!(lines(&packaged, "pkg/a.pyi"), ["3", "5"]);
    assert!(lines(&packaged, "a.py").is_empty());
    let stubs = sim112(Some(json!({ "path": { "regex": "\\.pyi$" } })));
    assert_eq!(lines(&stubs, "a.pyi"), ["3", "5"]);
    assert!(lines(&stubs, "a.py").is_empty());
    // `declaration` is TypeScript's; a Python rule cannot ask it.
    assert_eq!(
        rejection(compile(
            "python",
            json!({ "file": { "declaration": true }, "rule": { "kind": "subscript",
                    "resolves": { "imported": ["os"], "seenBefore": true } } })
        )),
        "/clauses/0/forbid/ast/file/declaration: declaration files are TypeScript's; python has none"
    );
}

#[test]
fn seen_before_lists_top_level_modules_alone() {
    for (rule, expected) in [
        (
            json!({ "kind": "call", "resolves": { "imported": ["os.path"], "seenBefore": true } }),
            "/clauses/0/forbid/ast/rule/resolves/imported/0: `os.path` is not a top-level module; `seenBefore` asks about modules by their first name, as Ruff's `seen_module` does",
        ),
        (
            json!({ "kind": "call", "resolves": { "seenBefore": true } }),
            "/clauses/0/forbid/ast/rule/resolves/seenBefore needs `imported` modules",
        ),
        (
            json!({ "kind": "call", "resolves": { "imported": ["os"], "seenBefore": true, "prefix": true } }),
            "/clauses/0/forbid/ast/rule/resolves/prefix: `seenBefore` asks whether an import of a listed module was seen; write other `resolves` terms in a resolves of their own",
        ),
        (
            json!({ "kind": "call", "resolves": { "imported": ["os"], "seenBefore": false } }),
            "/clauses/0/forbid/ast/rule/resolves/seenBefore must be true",
        ),
    ] {
        let error = compile("python", json!({ "rule": rule }))
            .err()
            .expect("rejected");
        assert_eq!(error.reason, expected, "{rule}");
    }
}

#[test]
fn malformed_python_versions_are_rejected() {
    let rule = json!({ "kind": "identifier", "resolves": { "global": ["print"] } });
    for (options, expected) in [
        (
            json!({ "pythonVersion": "3.6" }),
            "/clauses/0/forbid/ast/languageOptions/pythonVersion must be a version from \"3.7\" to \"3.15\"",
        ),
        (
            json!({ "pythonVersion": "3.16" }),
            "/clauses/0/forbid/ast/languageOptions/pythonVersion must be a version from \"3.7\" to \"3.15\"",
        ),
        (
            json!({ "pythonVersion": 3.13 }),
            "/clauses/0/forbid/ast/languageOptions/pythonVersion must be a version from \"3.7\" to \"3.15\"",
        ),
        (
            json!({ "pythonVersion": "3.013" }),
            "/clauses/0/forbid/ast/languageOptions/pythonVersion must be a version from \"3.7\" to \"3.15\"",
        ),
        (
            json!({ "ecmaVersion": 5 }),
            "unknown field `ecmaVersion` at /clauses/0/forbid/ast/languageOptions",
        ),
        (
            json!({ "typingModules": "compat" }),
            "/clauses/0/forbid/ast/languageOptions/typingModules must be a list of dotted names",
        ),
        (
            json!({ "typingModules": ["compat", "a..b"] }),
            "/clauses/0/forbid/ast/languageOptions/typingModules/1 must be a dotted name (identifiers joined by .) of at most 256 bytes",
        ),
        (
            json!({ "extendGenerics": ["django.db.models.$Key"] }),
            "/clauses/0/forbid/ast/languageOptions/extendGenerics/0 must be a dotted name (identifiers joined by .) of at most 256 bytes",
        ),
        (
            json!({ "extendGenerics": vec!["a"; 257] }),
            "/clauses/0/forbid/ast/languageOptions/extendGenerics lists 257 names; at most 256 are allowed",
        ),
    ] {
        let ast = json!({ "languageOptions": options, "rule": rule });
        assert_eq!(rejection(compile("python", ast.clone())), expected, "{ast}");
    }
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "languageOptions": { "pythonVersion": "3.13" }, "rule": rule })
        )),
        "unknown field `pythonVersion` at /clauses/0/forbid/ast/languageOptions"
    );
    assert_eq!(
        rejection(compile(
            "typescript",
            json!({ "languageOptions": { "typingModules": ["compat"] }, "rule": rule })
        )),
        "unknown field `typingModules` at /clauses/0/forbid/ast/languageOptions"
    );
    for (language, when, expected) in [
        (
            "python",
            json!([]),
            "/clauses/0/forbid/ast/when must be an object",
        ),
        (
            "python",
            json!({ "ecmaVersion": { "min": 5 } }),
            "unknown field `ecmaVersion` at /clauses/0/forbid/ast/when",
        ),
        (
            "typescript",
            json!({ "pythonVersion": { "min": "3.10" } }),
            "unknown field `pythonVersion` at /clauses/0/forbid/ast/when",
        ),
        (
            "python",
            json!({ "pythonVersion": "3.10" }),
            "/clauses/0/forbid/ast/when/pythonVersion must be an object",
        ),
        (
            "python",
            json!({ "pythonVersion": {} }),
            "/clauses/0/forbid/ast/when/pythonVersion must set `min`, `max` or both",
        ),
        (
            "python",
            json!({ "pythonVersion": { "min": "3.10", "exact": "3.11" } }),
            "unknown field `exact` at /clauses/0/forbid/ast/when/pythonVersion",
        ),
        (
            "python",
            json!({ "pythonVersion": { "min": "3.16" } }),
            "/clauses/0/forbid/ast/when/pythonVersion/min must be a version from \"3.7\" to \"3.15\"",
        ),
        (
            "python",
            json!({ "pythonVersion": { "min": "3.12", "max": "3.10" } }),
            "/clauses/0/forbid/ast/when/pythonVersion: min is after max, so the query never applies",
        ),
        (
            "typescript",
            json!({ "ecmaVersion": { "max": 4 } }),
            "/clauses/0/forbid/ast/when/ecmaVersion/max must be \"latest\", 3, 5, 6 to 17, or 2015 to 2026",
        ),
    ] {
        let ast = json!({ "when": when, "rule": { "kind": "identifier" } });
        assert_eq!(rejection(compile(language, ast.clone())), expected, "{ast}");
    }
}

#[test]
fn unresolved_matches_names_ruff_reports_undefined_or_star_imported() {
    let undefined =
        python(json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": true } } }));
    let star =
        python(json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": "star" } } }));
    let source = "\
import os
print(os, missing)
x = 1
del x
x
len = 1
del len
len
class C:
    __qualname__, __module__, __class__
    def m(self):
        return __class__, __qualname__
    f = lambda: __class__
def g():
    return __class__, __path__
def later():
    global declared_later
declared_later
from mod import *
starred, os
def h():
    return early
def early_star():
    pass
";
    let result = evaluate_file(&undefined, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["2:missing", "5:x", "8:len", "10:__class__"],
        "{result}"
    );
    // Ruff 0.16.9: function bodies are read after the module, so they see the star import.
    let result = evaluate_file(&star, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        [
            "12:__qualname__",
            "15:__class__",
            "15:__path__",
            "20:starred",
            "22:early"
        ],
        "{result}"
    );
    // `__path__` is bound in a package's `__init__`.
    let result = evaluate_module(
        &undefined,
        "pkg/__init__.py",
        Some("pkg.__init__"),
        "__path__\n",
    );
    assert_eq!(
        found(&result, "__path__\n"),
        Vec::<String>::new(),
        "{result}"
    );
    // The builtins are the query's version's (Ruff's default target, 3.10, without options).
    let source = "aiter\nExceptionGroup\n";
    let result = evaluate_file(&undefined, "python", "a.py", source);
    assert_eq!(found(&result, source), ["2:ExceptionGroup"], "{result}");
    let at_311 = python(json!({
        "languageOptions": { "pythonVersion": "3.11" },
        "rule": { "kind": "identifier", "resolves": { "unresolved": true } },
    }));
    let result = evaluate_file(&at_311, "python", "a.py", source);
    assert_eq!(found(&result, source), Vec::<String>::new(), "{result}");
    // A star import in a class body is invisible to its methods; a function body sees one made
    // after it in the module.
    let source = "class K:\n    from m import *\n    a\n    def f(self):\n        return b\n";
    let result = evaluate_file(&star, "python", "a.py", source);
    assert_eq!(found(&result, source), ["3:a"], "{result}");
    let result = evaluate_file(&undefined, "python", "a.py", source);
    assert_eq!(found(&result, source), ["5:b"], "{result}");
}

#[test]
fn unresolved_typescript_names_are_no_undef_s() {
    let rule = typescript(json!({ "rule": { "resolves": { "unresolved": true } } }));
    let source = "let a = 1;\nconsole.log(a, b, Array, window);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        found(&result, source),
        ["2:console", "2:b", "2:window"],
        "{result}"
    );
    let browser = typescript(json!({
        "languageOptions": { "globals": { "window": "readonly" } },
        "rule": { "resolves": { "unresolved": true, "directives": "eslint" } },
    }));
    let source = "/* global b */\nb(c, window);\n";
    let result = evaluate_file(&browser, "typescript", "a.ts", source);
    assert_eq!(found(&result, source), ["2:c"], "{result}");
}

#[test]
fn global_true_matches_any_unshadowed_declared_global_or_builtin() {
    let rule = python(json!({ "rule": { "kind": "identifier", "resolves": { "global": true } } }));
    let source = "print(len, missing)\ndef f(len):\n    return len, open\n";
    let result = evaluate_file(&rule, "python", "a.py", source);
    assert_eq!(
        found(&result, source),
        ["1:print", "1:len", "3:open"],
        "{result}"
    );
    let rule = typescript(json!({ "rule": { "resolves": { "global": true } } }));
    let source = "const Map = 1;\nconsole.log(Map, Set, window);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    // `console` is no ECMAScript global: ESLint declares it only with an environment's globals.
    assert_eq!(found(&result, source), ["2:Set"], "{result}");
}

#[test]
fn import_true_matches_names_with_a_qualified_name() {
    let rule = python(json!({ "rule": {
        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
        "not": { "inside": { "kind": "attribute", "stopBy": "neighbor" } },
        "resolves": { "import": true },
    } }));
    let source = "\
import os.path
from . import rel
def local(): pass
value = 1
os.path.join(value)
rel.x
local()
print(missing)
len = 2
len
";
    let result = evaluate_file(&rule, "python", "m.py", source);
    assert_eq!(
        found(&result, source),
        ["5:os.path.join", "7:local", "8:print"],
        "{result}"
    );
    let result = evaluate_module(&rule, "pkg/m.py", Some("pkg.m"), source);
    assert_eq!(
        found(&result, source),
        ["5:os.path.join", "6:rel.x", "7:local", "8:print"],
        "{result}"
    );
}

#[test]
fn any_mode_resolves_are_checked() {
    for (ast, expected) in [
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/unresolved must be true or \"star\"",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": true, "global": ["a"] } } }),
            "/clauses/0/forbid/ast/rule/resolves/global: `unresolved` asks about names with no binding; write other `resolves` terms in a resolves of their own",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "import": true, "prefix": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/prefix: `import: true` matches every qualified name; it lists none to prefix",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": true, "declared": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/declared: `global: true` asks for declared globals and builtins; it has no names to undeclare",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "global": false } } }),
            "/clauses/0/forbid/ast/rule/resolves/global must be true or an array of names",
        ),
        (
            json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": true, "directives": "eslint" } } }),
            "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments are ESLint's; Python has none",
        ),
    ] {
        assert_eq!(rejection(compile("python", ast.clone())), expected, "{ast}");
    }
    for (ast, expected) in [
        (
            json!({ "rule": { "resolves": { "unresolved": "star" } } }),
            "/clauses/0/forbid/ast/rule/resolves/unresolved: \"star\" asks whether a Python star import binds the name; TypeScript has none",
        ),
        (
            json!({ "rule": { "resolves": { "import": true } } }),
            "/clauses/0/forbid/ast/rule/resolves/import: import resolution is Python-only for now; TypeScript rules can use `global`",
        ),
    ] {
        assert_eq!(
            rejection(compile("typescript", ast.clone())),
            expected,
            "{ast}"
        );
    }
}

/// SQL refuses the `resolves` keys only other families answer, each at its own key, before the
/// key's companions (`this`, `global`, `imported`) are compiled.
#[test]
fn sql_refuses_resolves_keys_of_other_families() {
    for (resolves, expected) in [
        (
            json!({ "this": "global", "sloppyFunctions": true }),
            "/clauses/0/forbid/ast/rule/resolves/sloppyFunctions: sloppy-mode functions are JavaScript's; SQL has none",
        ),
        (
            json!({ "global": ["a"], "declared": false }),
            "/clauses/0/forbid/ast/rule/resolves/declared: declared globals come from ESLint's and Ruff's configuration; SQL has none",
        ),
        (
            json!({ "imported": ["a"], "relative": "text" }),
            "/clauses/0/forbid/ast/rule/resolves/relative: relative imports are Python's; a SQL file imports nothing",
        ),
        (
            json!({ "imported": ["os"], "seenBefore": true }),
            "/clauses/0/forbid/ast/rule/resolves/seenBefore: Ruff's `seen_module` reads Python's imports; a SQL file imports nothing",
        ),
    ] {
        let ast = json!({ "rule": { "kind": "identifier", "resolves": resolves } });
        assert_eq!(rejection(compile("sql", ast.clone())), expected, "{ast}");
    }
}

#[test]
fn any_mode_resolves_are_source_incomplete_under_syntax_errors() {
    for resolves in [
        json!({ "unresolved": true }),
        json!({ "global": true }),
        json!({ "import": true }),
    ] {
        let rule = python(json!({ "rule": { "kind": "identifier", "resolves": resolves } }));
        let result = evaluate_file(&rule, "python", "a.py", "print(x)\ndef f(:\n");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
    }
}

/// A syntax error leaves a stateless evaluation incomplete only in a file the query's file gates
/// admit: as `run_clause` reads it, a file they exclude matches nothing, errors or not.
#[test]
fn a_syntax_error_is_no_failure_where_the_file_gates_exclude_the_file() {
    // A negated outward relation: unsound under syntax errors, so an admitted errored file fails.
    let gated = |file: Option<Value>| {
        let mut ast = json!({ "when": { "sourceType": "commonjs" },
            "rule": { "kind": "identifier",
                "not": { "inside": { "kind": "function_declaration", "stopBy": "end" } } } });
        if let Some(file) = file {
            ast["file"] = file;
        }
        typescript(ast)
    };
    let incomplete = |rule: &CompiledCodeRuleDsl, files: &[(&str, &str)]| {
        let files = files
            .iter()
            .map(|(path, source)| {
                let language =
                    crate::code_rule_dsl::language::label_of_path(path).unwrap_or("json");
                json!({ "language": language, "path": path, "source": source })
            })
            .collect::<Vec<_>>();
        let snapshot = json!({
            "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0,
        });
        let result: Value = serde_json::from_str(
            &rule
                .evaluate(snapshot.to_string(), "report".to_owned())
                .expect("evaluation succeeds"),
        )
        .expect("evaluation is JSON");
        result["failures"]
            .as_array()
            .expect("failures")
            .iter()
            .filter(|failure| failure["code"] == "source-incomplete")
            .map(|failure| failure["path"].as_str().expect("a path").to_owned())
            .collect::<Vec<_>>()
    };
    // Node's syntax detection makes esm.js a module, which the commonjs-gated query skips.
    let typeless = [
        ("package.json", r#"{ "name": "a" }"#),
        ("esm.js", "import x from 'x';\nfunction f() { x(; }\n"),
        ("cjs.js", "require('x');\nfunction f() { x(; }\n"),
    ];
    assert_eq!(incomplete(&gated(None), &typeless), ["cjs.js"]);
    assert_eq!(
        incomplete(&gated(None), &typeless[..2]),
        Vec::<String>::new()
    );
    // A file whose JSX runtime the query does not list is skipped too.
    let react = gated(Some(json!({ "jsxRuntime": ["react"] })));
    assert_eq!(
        incomplete(
            &react,
            &[("a.cjs", "/** @jsxImportSource preact */\nx(;\n")]
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        incomplete(&react, &[("a.cjs", "/** @jsxImportSource react */\nx(;\n")]),
        ["a.cjs"]
    );
}

/// The paths among `files` (path and source; a `package.json` is a JSON entry) that a TypeScript
/// query over each file's `program` with this `file` condition matches, in a stateless snapshot.
/// A `package` given with a file is the host's.
fn matched_paths(file: Value, files: &[(&str, &str, Option<Value>)]) -> Vec<String> {
    let rule = typescript(json!({ "file": file, "rule": { "kind": "program" } }));
    let files = files
        .iter()
        .map(|(path, source, package)| {
            let language = crate::code_rule_dsl::language::label_of_path(path).unwrap_or("json");
            let mut file = json!({ "language": language, "path": path, "source": source });
            if let Some(package) = package {
                file["package"] = package.clone();
            }
            file
        })
        .collect::<Vec<_>>();
    let snapshot = json!({
        "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0,
    });
    let result: Value = serde_json::from_str(
        &rule
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect("evaluation succeeds"),
    )
    .expect("evaluation is JSON");
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut paths = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| violation["path"].as_str().expect("a path").to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

/// `matched_paths` over files that carry no host `package`.
fn matched(file: Value, files: &[(&str, &str)]) -> Vec<String> {
    let files = files
        .iter()
        .map(|(path, source)| (*path, *source, None))
        .collect::<Vec<_>>();
    matched_paths(file, &files)
}

#[test]
fn file_package_bin_reads_the_manifests_bin_targets() {
    let bin = |value: bool| json!({ "package": { "bin": value } });
    // The string form, resolved against the manifest's folder like eslint-plugin-n's `isBinFile`:
    // the target itself, or the file without `.js`, or without `.js` and `/index`.
    let string = [
        ("package.json", r#"{ "bin": "./cli.js" }"#),
        ("cli.js", ""),
        ("lib.js", ""),
        ("tools/package.json", r#"{ "bin": "bin" }"#),
        ("tools/bin/index.js", ""),
        ("tools/bin.js", ""),
        ("tools/bin/other.js", ""),
    ];
    assert_eq!(
        matched(bin(true), &string),
        ["cli.js", "tools/bin.js", "tools/bin/index.js"]
    );
    assert_eq!(
        matched(bin(false), &string),
        ["lib.js", "tools/bin/other.js"]
    );
    // The object form: every string value, resolved through `..` as `path.resolve` does; one that
    // ends outside the folder or is absolute names nothing below it.
    let object = [
        (
            "pkg/package.json",
            r#"{ "bin": { "a": "./bin/a", "b": "lib/../b.js", "c": "../pkg/c.js", "d": "/pkg/d.js", "e": "../d.js", "f": 3 } }"#,
        ),
        ("pkg/bin/a.js", ""),
        ("pkg/b.js", ""),
        ("pkg/c.js", ""),
        ("pkg/d.js", ""),
    ];
    assert_eq!(
        matched(bin(true), &object),
        ["pkg/b.js", "pkg/bin/a.js", "pkg/c.js"]
    );
    assert_eq!(matched(bin(false), &object), ["pkg/d.js"]);
    // Without a manifest neither holds, as eslint-plugin-n's hashbang rule reports nothing there.
    let bare = [("cli.js", "")];
    assert!(matched(bin(true), &bare).is_empty());
    assert!(matched(bin(false), &bare).is_empty());
    // The nearest manifest decides, whatever a farther one lists.
    let nested = [
        ("package.json", r#"{ "bin": "sub/cli.js" }"#),
        ("sub/package.json", "{}"),
        ("sub/cli.js", ""),
    ];
    assert_eq!(matched(bin(false), &nested), ["sub/cli.js"]);
}

#[test]
fn file_package_depends_on_and_path_read_the_nearest_manifest() {
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        ("app/page.tsx", ""),
        (
            "apps/web/package.json",
            r#"{ "peerDependencies": { "next": "15" } }"#,
        ),
        ("apps/web/app/page.tsx", ""),
        (
            "apps/vite/package.json",
            r#"{ "devDependencies": { "vite": "7" } }"#,
        ),
        ("apps/vite/app/page.tsx", ""),
        ("apps/broken/package.json", "{"),
        ("apps/broken/app/page.tsx", ""),
    ];
    let next = json!({ "package": { "dependsOn": ["next", "nuxt"] } });
    assert_eq!(
        matched(next, &files),
        ["app/page.tsx", "apps/web/app/page.tsx"]
    );
    // `path` is relative to the manifest's folder.
    let app = json!({ "package": { "path": { "regex": "^app/page\\.tsx$" } } });
    assert_eq!(
        matched(app.clone(), &files),
        [
            "app/page.tsx",
            "apps/broken/app/page.tsx",
            "apps/vite/app/page.tsx",
            "apps/web/app/page.tsx"
        ]
    );
    let next_app = json!({ "package": { "dependsOn": ["next"], "path": { "regex": "^app/" } } });
    assert_eq!(
        matched(next_app, &files),
        ["app/page.tsx", "apps/web/app/page.tsx"]
    );
    // Without a manifest `dependsOn` and `path` fail, `notRegex` included.
    let bare = [("app/page.tsx", "")];
    assert!(matched(json!({ "package": { "dependsOn": ["next"] } }), &bare).is_empty());
    assert!(matched(app, &bare).is_empty());
    assert!(matched(json!({ "package": { "path": { "notRegex": "x" } } }), &bare).is_empty());
}

#[test]
fn file_package_node_version_reads_eslint_plugin_ns_configured_range() {
    let subset_of = |range: &str| json!({ "package": { "nodeVersion": { "subsetOf": range } } });
    let at = |manifest: &str, range: &str| {
        !matched(
            subset_of(range),
            &[("package.json", manifest), ("a.js", "")],
        )
        .is_empty()
    };
    // `engines.node`.
    assert!(at(r#"{ "engines": { "node": ">=18.18" } }"#, ">=18"));
    assert!(!at(r#"{ "engines": { "node": ">=18.18" } }"#, ">=20"));
    assert!(at(r#"{ "engines": { "node": "^20 || ^22" } }"#, ">=20.0.0"));
    // `devEngines.runtime` (one entry or a list, the first named `node` with a string version)
    // where `engines.node` is absent or not a range.
    let dev = r#"{ "devEngines": { "runtime": { "name": "node", "version": ">=22" } } }"#;
    assert!(at(dev, ">=20"));
    let listed = r#"{ "engines": { "node": "latest" },
        "devEngines": { "runtime": [{ "name": "bun", "version": "1" }, { "name": "node", "version": "22.x" }] } }"#;
    assert!(at(listed, ">=22"));
    // `engines.node` wins over `devEngines`; a range neither gives is eslint-plugin-n's
    // `>=16.0.0`, also without a manifest.
    let both = r#"{ "engines": { "node": ">=14" }, "devEngines": { "runtime": { "name": "node", "version": ">=22" } } }"#;
    assert!(!at(both, ">=20"));
    for manifest in ["{}", r#"{ "engines": { "node": "lts" } }"#, "["] {
        assert!(at(manifest, ">=16"), "{manifest}");
        assert!(!at(manifest, ">=18"), "{manifest}");
    }
    assert!(!matched(subset_of(">=16"), &[("a.js", "")]).is_empty());
    assert!(matched(subset_of(">=18"), &[("a.js", "")]).is_empty());
}

/// A host's `package` must send `nodeVersion` as a valid npm range: one that does not parse is
/// malformed, not eslint-plugin-n's `>=16.0.0` default in disguise; a valid one is kept.
#[test]
fn a_host_package_node_version_must_be_a_valid_npm_range() {
    let file = json!({ "package": { "nodeVersion": { "subsetOf": ">=18" } } });
    let package =
        |node_version: &str| json!({ "folder": "", "type": "unset", "nodeVersion": node_version });
    assert_eq!(
        matched_paths(file.clone(), &[("a.js", "", Some(package(">=20")))]),
        ["a.js"]
    );
    assert_eq!(
        matched_paths(file, &[("a.js", "", Some(package(">=16")))]),
        Vec::<String>::new()
    );
    let rule = typescript(
        json!({ "file": { "package": { "bin": false } }, "rule": { "kind": "program" } }),
    );
    let snapshot = json!({
        "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0,
        "files": [{ "language": "javascript", "path": "a.js", "source": "",
            "package": package("^18.x.y") }],
    });
    let error = rule
        .evaluate(snapshot.to_string(), "report".to_owned())
        .expect_err("an invalid nodeVersion is malformed");
    assert!(
        error
            .reason
            .contains("\"nodeVersion\"?: string (a valid npm range)"),
        "{}",
        error.reason
    );
}

/// An editor's edit of a `package.json` gives the files whose `package` the snapshot derived
/// theirs again, as a fresh snapshot of the same final texts reads them: its `type`, engines,
/// `bin` and dependencies alike, for the files below it only. A host's own `package` stays.
#[test]
fn an_editor_edit_of_a_package_json_rederives_the_packages_below_it() {
    let clause = |id: &str, ast: Value| {
        json!({ "id": id, "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": id.to_uppercase(), "message": id } })
    };
    let program = |id: &str, package: Value| {
        clause(
            id,
            json!({ "language": "typescript", "file": { "package": package }, "rule": { "kind": "program" } }),
        )
    };
    let compiled = compile_document_with(
        json!({ "format": "rerail.policy/v1", "id": "editor-manifests", "revision": 1, "mode": "block",
            "clauses": [
                clause("require", json!({ "language": "typescript", "languageOptions": {},
                    "rule": { "kind": "identifier", "resolves": { "global": ["require"] } } })),
                program("bin", json!({ "bin": true })),
                program("node", json!({ "nodeVersion": { "subsetOf": ">=20" } })),
                program("next", json!({ "dependsOn": ["next"] })),
            ] }),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "module.exports = require('x');\n";
    // A `.ts` file reads its package as a `.js` one does; its type-only import is stripped.
    let typed = "import type { A } from 'a';\nmodule.exports = require('x');\n";
    let snapshot = |manifest: &str| {
        json!({ "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0, "files": [
            { "language": "json", "path": "package.json", "source": manifest },
            { "language": "javascript", "path": "lib/plain.js", "source": source },
            { "language": "typescript", "path": "lib/typed.ts", "source": typed },
            { "language": "json", "path": "sub/package.json", "source": r#"{ "type": "commonjs" }"# },
            { "language": "javascript", "path": "sub/nested.js", "source": source },
            { "language": "javascript", "path": "host.js", "source": source,
                "package": { "folder": "", "type": "commonjs", "bin": ["host.js"] } },
        ] })
        .to_string()
    };
    // Each violation as `path CODE`, sorted.
    let reported = |result: &str| {
        let result: Value = serde_json::from_str(result).expect("evaluation is JSON");
        assert_eq!(result["failures"], json!([]), "{result}");
        let mut violations = result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| {
                format!(
                    "{} {}",
                    violation["path"].as_str().expect("a path"),
                    violation["code"].as_str().expect("a code")
                )
            })
            .collect::<Vec<_>>();
        violations.sort();
        violations
    };
    // The host's file and the one below the nearer manifest never change.
    let expected = |plain: &[&str]| {
        let mut expected = ["host.js BIN", "host.js REQUIRE", "sub/nested.js REQUIRE"]
            .iter()
            .chain(plain)
            .map(|violation| (*violation).to_owned())
            .collect::<Vec<_>>();
        expected.sort();
        expected
    };
    let mut current = r#"{ "type": "commonjs" }"#.to_owned();
    let mut editor = compiled.open_editor(snapshot(&current)).unwrap();
    assert_eq!(
        reported(&editor.evaluate("report".to_owned()).unwrap()),
        expected(&["lib/plain.js REQUIRE", "lib/typed.ts REQUIRE"])
    );
    let edits: [(&str, &[&str]); 5] = [
        (r#"{ "type": "module" }"#, &[]),
        (
            r#"{ "type": "module", "engines": { "node": ">=22" } }"#,
            &["lib/plain.js NODE", "lib/typed.ts NODE"],
        ),
        (
            r#"{ "type": "module", "engines": { "node": ">=22" }, "bin": "./lib/plain.js", "dependencies": { "next": "15" } }"#,
            &[
                "lib/plain.js BIN",
                "lib/plain.js NEXT",
                "lib/plain.js NODE",
                "lib/typed.ts NEXT",
                "lib/typed.ts NODE",
            ],
        ),
        // No `type`: Node's detection reads both files as CommonJS.
        ("{}", &["lib/plain.js REQUIRE", "lib/typed.ts REQUIRE"]),
        (
            r#"{ "type": "commonjs" }"#,
            &["lib/plain.js REQUIRE", "lib/typed.ts REQUIRE"],
        ),
    ];
    for (revision, (manifest, plain)) in (1..).zip(edits) {
        let edit = json!({ "revision": revision, "path": "package.json", "startByte": 0,
            "oldEndByte": current.len(), "newText": manifest });
        let edited = editor
            .apply_edit(edit.to_string(), "report".to_owned())
            .unwrap();
        let fresh = compiled
            .evaluate(snapshot(manifest), "report".to_owned())
            .unwrap();
        assert_eq!(reported(&edited), expected(plain), "{manifest}");
        assert_eq!(reported(&fresh), expected(plain), "{manifest}");
        current = manifest.to_owned();
    }
}

/// `file.package.serverGraph` reads the module graph of the file set's Next app: a
/// module only a `"use client"` module imports is client code. An editor edit of a source the
/// graph reads gives the same answer as a fresh snapshot; a host's own fact stays.
#[test]
fn file_package_server_graph_reads_the_next_apps_module_graph() {
    let program = |id: &str, member: bool| {
        json!({ "id": id, "violation": { "anchor": "witness.span", "code": id.to_uppercase(), "message": id },
            "forbid": { "ast": { "language": "typescript",
                "file": { "package": { "serverGraph": member } }, "rule": { "kind": "program" } } } })
    };
    let compiled = compile_document_with(
        json!({ "format": "rerail.policy/v1", "id": "server-graph", "revision": 1, "mode": "block",
            "clauses": [program("server", true), program("client", false)] }),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let widget = "'use client';\nimport { useThing } from './adapters/react';\nexport default function Widget() { useThing(); }\n";
    let snapshot = |widget: &str| {
        json!({ "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0, "files": [
            { "language": "json", "path": "package.json", "source": r#"{ "dependencies": { "next": "15" } }"# },
            { "language": "tsx", "path": "app/page.tsx", "source": "import Widget from './widget';\nexport default function Page() {}\n" },
            { "language": "tsx", "path": "app/widget.tsx", "source": widget },
            { "language": "typescript", "path": "app/adapters/react.ts", "source": "import { useState } from 'react';\nexport const useThing = () => useState(0);\n" },
            // A package that names no `next` has no graph: neither value holds.
            { "language": "json", "path": "vite/package.json", "source": "{}" },
            { "language": "tsx", "path": "vite/app/page.tsx", "source": "export default 1;\n" },
            // A host's fact for its own file.
            { "language": "typescript", "path": "host.ts", "source": "export {};\n",
                "package": { "folder": "", "type": "unset", "dependencies": ["next"], "serverGraph": true } },
        ] })
        .to_string()
    };
    let reported = |result: &str| {
        let result: Value = serde_json::from_str(result).expect("evaluation is JSON");
        assert_eq!(result["failures"], json!([]), "{result}");
        let mut violations = result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| {
                format!(
                    "{} {}",
                    violation["path"].as_str().expect("a path"),
                    violation["code"].as_str().expect("a code")
                )
            })
            .collect::<Vec<_>>();
        violations.sort();
        violations
    };
    let client = [
        "app/adapters/react.ts CLIENT",
        "app/page.tsx SERVER",
        "app/widget.tsx CLIENT",
        "host.ts SERVER",
    ];
    assert_eq!(
        reported(
            &compiled
                .evaluate(snapshot(widget), "report".to_owned())
                .unwrap()
        ),
        client
    );
    // Without its directive the widget, and the module it imports, join the server graph.
    let mut editor = compiled.open_editor(snapshot(widget)).unwrap();
    let server = [
        "app/adapters/react.ts SERVER",
        "app/page.tsx SERVER",
        "app/widget.tsx SERVER",
        "host.ts SERVER",
    ];
    let directive = "'use client';\n";
    let edit = json!({ "revision": 1, "path": "app/widget.tsx", "startByte": 0,
        "oldEndByte": directive.len(), "newText": "" });
    let edited = editor
        .apply_edit(edit.to_string(), "report".to_owned())
        .unwrap();
    assert_eq!(reported(&edited), server);
    assert_eq!(
        reported(
            &compiled
                .evaluate(snapshot(&widget[directive.len()..]), "report".to_owned())
                .unwrap()
        ),
        server
    );
}

#[test]
fn file_jsx_runtime_reads_pragmas_then_imports_then_the_manifest() {
    let runtime = |runtimes: &[&str], manifest: Option<&str>, source: &str| {
        let mut files = vec![("a.tsx", source)];
        if let Some(manifest) = manifest {
            files.push(("package.json", manifest));
        }
        !matched(json!({ "jsxRuntime": runtimes }), &files).is_empty()
    };
    let react = Some(r#"{ "dependencies": { "react": "19", "react-dom": "19" } }"#);
    let preact = Some(r#"{ "dependencies": { "preact": "10" } }"#);
    let both = Some(r#"{ "dependencies": { "next": "15" }, "devDependencies": { "vue": "3" } }"#);
    let plain = "export const a = <div />;\n";
    // The manifest: React's packages alone, another library's alone, both or neither.
    assert!(runtime(&["react"], react, plain));
    assert!(runtime(&["other"], preact, plain));
    assert!(runtime(&["unknown"], both, plain));
    assert!(runtime(&["unknown"], Some("{}"), plain));
    assert!(runtime(&["unknown"], None, plain));
    assert!(!runtime(&["react", "other"], None, plain));
    // A static import of another library's `h` or `createElement` beats the manifest.
    let preact_h = "import { h, Fragment } from 'preact';\nexport const a = <div />;\n";
    assert!(runtime(&["other"], react, preact_h));
    assert!(runtime(
        &["other"],
        None,
        "import { createElement as c } from \"vue\";\n"
    ));
    assert!(runtime(
        &["react"],
        react,
        "import { render } from 'preact';\n"
    ));
    // A pragma beats both; `@jsxImportSource` beats `@jsx`.
    let pragma = |comment: &str| format!("{comment}\n{preact_h}");
    assert!(runtime(
        &["react"],
        preact,
        &pragma("/** @jsx React.createElement */")
    ));
    assert!(runtime(&["other"], react, &pragma("/* @jsx h */")));
    assert!(runtime(
        &["react"],
        None,
        &pragma("// @jsxImportSource react/jsx-runtime")
    ));
    // A React wrapper's import source is React's, even over another library's import.
    for source in ["@emotion/react", "theme-ui", "react/jsx-dev-runtime"] {
        let comment = format!("/** @jsxImportSource {source} */");
        assert!(runtime(&["react"], preact, &pragma(&comment)), "{source}");
    }
    for source in [
        "preact",
        "solid-js/h",
        "vue",
        "@builder.io/qwik",
        "hono/jsx",
    ] {
        let comment = format!("/** @jsxImportSource {source} */\n{plain}");
        assert!(runtime(&["other"], react, &comment), "{source}");
    }
    assert!(runtime(
        &["react"],
        preact,
        &pragma("/* @jsx h */\n/* @jsxImportSource react */")
    ));
    // Any other pragma is no evidence: the imports and then the manifest decide, so Emotion's
    // classic `@jsx jsx` in a React package is React's and `@jsx h` with Preact's `h` is not.
    let emotion =
        "/** @jsx jsx */\nimport { jsx } from '@emotion/react';\nexport const a = <div />;\n";
    assert!(runtime(&["react"], react, emotion));
    assert!(runtime(&["unknown"], None, emotion));
    assert!(runtime(
        &["react"],
        react,
        &format!("/** @jsxImportSource @acme/jsx */\n{plain}")
    ));
}

/// A JSX pragma is read in its comment's value, without the `//`, `/*` and `*/` around it
/// (Babel's `comment.value`), however tightly they close it.
#[test]
fn file_jsx_runtime_reads_a_pragma_without_its_comment_delimiters() {
    let runtime = |runtimes: &[&str], manifest: &str, source: &str| {
        !matched(
            json!({ "jsxRuntime": runtimes }),
            &[("a.tsx", source), ("package.json", manifest)],
        )
        .is_empty()
    };
    let react = r#"{ "dependencies": { "react": "19" } }"#;
    let preact = r#"{ "dependencies": { "preact": "10" } }"#;
    let plain = "export const a = <div />;\n";
    for comment in [
        "/* @jsxImportSource preact*/",
        "/*@jsxImportSource preact*/",
        "//@jsxImportSource preact",
    ] {
        assert!(
            runtime(&["other"], react, &format!("{comment}\n{plain}")),
            "{comment}"
        );
    }
    assert!(runtime(
        &["react"],
        preact,
        &format!("/*@jsx React*/\n{plain}")
    ));
    assert!(runtime(
        &["react"],
        preact,
        &format!("/*@jsx React.createElement*/\n{plain}")
    ));
}

/// The package's `tsconfig.json` (else `jsconfig.json`) `jsxImportSource` names its JSX runtime
/// before its dependencies do, through relative `extends`; a file's own pragma or imports still
/// win. An editor edit of the configuration gives the fresh snapshot's answer.
#[test]
fn file_jsx_runtime_reads_the_packages_jsx_import_source() {
    let mixed = r#"{ "dependencies": { "react": "19", "preact": "10" } }"#;
    let plain = "export const a = <div />;\n";
    let runtime = |runtimes: &[&str], files: &[(&str, &str)]| {
        !matched(json!({ "jsxRuntime": runtimes }), files).is_empty()
    };
    let config = |source: &str| {
        format!(
            "{{\n  // the JSX transform\n  \"compilerOptions\": {{ \"jsx\": \"react-jsx\", \"jsxImportSource\": \"{source}\", }},\n}}\n"
        )
    };
    let preact = config("preact");
    let emotion = config("@emotion/react");
    // React and Preact both listed: the manifest alone says neither.
    assert!(runtime(
        &["unknown"],
        &[("a.tsx", plain), ("package.json", mixed)]
    ));
    assert!(runtime(
        &["other"],
        &[
            ("a.tsx", plain),
            ("package.json", mixed),
            ("tsconfig.json", &preact)
        ]
    ));
    assert!(runtime(
        &["react"],
        &[
            ("a.tsx", plain),
            ("package.json", mixed),
            ("jsconfig.json", &emotion)
        ]
    ));
    assert!(runtime(
        &["other"],
        &[
            ("a.tsx", plain),
            ("package.json", mixed),
            ("tsconfig.json", r#"{ "extends": "./tsconfig.base" }"#),
            ("tsconfig.base.json", &preact),
        ]
    ));
    // A source naming no known runtime falls through to the dependencies; the file's pragma wins.
    assert!(runtime(
        &["unknown"],
        &[
            ("a.tsx", plain),
            ("package.json", mixed),
            ("tsconfig.json", &config("@acme/jsx"))
        ]
    ));
    assert!(runtime(
        &["react"],
        &[
            ("a.tsx", &format!("/** @jsxImportSource react */\n{plain}")),
            ("package.json", mixed),
            ("tsconfig.json", &preact),
        ]
    ));
    let compiled = compile_document_with(
        json!({ "format": "rerail.policy/v1", "id": "jsx-import-source", "revision": 1, "mode": "block",
            "clauses": [{ "id": "other", "violation": { "anchor": "witness.span", "code": "OTHER", "message": "other" },
                "forbid": { "ast": { "language": "typescript", "file": { "jsxRuntime": ["other"] }, "rule": { "kind": "program" } } } }] }),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let snapshot = |config: &str| {
        json!({ "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0, "files": [
            { "language": "json", "path": "package.json", "source": mixed },
            { "language": "json", "path": "tsconfig.json", "source": config },
            { "language": "tsx", "path": "a.tsx", "source": plain },
        ] })
        .to_string()
    };
    let count = |result: &str| {
        serde_json::from_str::<Value>(result).expect("JSON")["violations"]
            .as_array()
            .expect("violations")
            .len()
    };
    let mut editor = compiled.open_editor(snapshot(&emotion)).unwrap();
    assert_eq!(count(&editor.evaluate("report".to_owned()).unwrap()), 0);
    let edit = json!({ "revision": 1, "path": "tsconfig.json", "startByte": 0,
        "oldEndByte": emotion.len(), "newText": preact });
    assert_eq!(
        count(
            &editor
                .apply_edit(edit.to_string(), "report".to_owned())
                .unwrap()
        ),
        1
    );
    assert_eq!(
        count(
            &compiled
                .evaluate(snapshot(&preact), "report".to_owned())
                .unwrap()
        ),
        1
    );
}

#[test]
fn a_snapshot_derives_package_facts_from_its_package_json_entries_unless_the_host_sends_them() {
    let next = json!({ "package": { "dependsOn": ["next"] } });
    let manifest = r#"{ "dependencies": { "next": "15" } }"#;
    // Derived from the entry, as a workspace holding the same files does.
    assert_eq!(
        matched(next.clone(), &[("package.json", manifest), ("a.ts", "")]),
        ["a.ts"]
    );
    // A host's `package` wins over the entries, and without either the file has no manifest.
    let host = json!({ "folder": "", "type": "unset", "dependencies": ["react"] });
    assert!(
        matched_paths(
            next.clone(),
            &[
                ("package.json", manifest, None),
                ("a.ts", "", Some(host.clone()))
            ]
        )
        .is_empty()
    );
    let host_next = json!({ "folder": "", "type": "unset", "dependencies": ["next"] });
    assert_eq!(
        matched_paths(next.clone(), &[("a.ts", "", Some(host_next))]),
        ["a.ts"]
    );
    assert!(matched(next, &[("a.ts", "")]).is_empty());
    // The entry's `type` decides a `.js` file's module system too.
    let commonjs = with_options(&["require"], json!({}));
    let evaluate = |files: Value| -> Value {
        let snapshot = json!({
            "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0,
        });
        serde_json::from_str(
            &commonjs
                .evaluate(snapshot.to_string(), "report".to_owned())
                .expect("evaluation succeeds"),
        )
        .expect("evaluation is JSON")
    };
    let source = "require;\n";
    let file = json!({ "language": "javascript", "path": "lib/a.js", "source": source });
    let typed = |package_type: &str| json!({ "language": "json", "path": "package.json", "source": format!("{{ \"type\": \"{package_type}\" }}") });
    assert_eq!(
        found(&evaluate(json!([typed("commonjs"), file.clone()])), source),
        ["1:require"]
    );
    assert!(found(&evaluate(json!([typed("module"), file.clone()])), source).is_empty());
    assert!(found(&evaluate(json!([file])), source).is_empty());
}

#[test]
fn module_system_package_type_reads_extension_and_package_type_alone() {
    // `require` is a configured global only where the query reads the file as CommonJS.
    let read = |module_system: &str, files: Value, path: &str| -> bool {
        let rule = with_options(&["require"], json!({ "moduleSystem": module_system }));
        let snapshot = json!({
            "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0,
        });
        let report: Value = serde_json::from_str(
            &rule
                .evaluate(snapshot.to_string(), "report".to_owned())
                .expect("evaluation succeeds"),
        )
        .expect("evaluation is JSON");
        report["violations"]
            .as_array()
            .is_some_and(|violations| violations.iter().any(|violation| violation["path"] == path))
    };
    let manifest =
        |source: &str| json!({ "language": "json", "path": "package.json", "source": source });
    let file = |path: &str, source: &str| {
        let language = if path.ends_with("ts") {
            "typescript"
        } else {
            "javascript"
        };
        json!({ "language": language, "path": path, "source": source })
    };
    // A `.ts` file with module syntax under a type-less manifest: Node's syntax detection makes it
    // a module (the default), its package type CommonJS.
    let esm = "import x from \"x\";\nrequire;\n";
    let untyped = json!([manifest("{}"), file("src/a.ts", esm)]);
    assert!(!read("file", untyped.clone(), "src/a.ts"));
    assert!(read("package-type", untyped, "src/a.ts"));
    // Without any manifest: a module by default, CommonJS by package type.
    let bare = json!([file("legacy.js", "require;\n")]);
    assert!(!read("file", bare.clone(), "legacy.js"));
    assert!(read("package-type", bare, "legacy.js"));
    // `type: module` and the extensions decide the same either way.
    let typed = json!([
        manifest("{ \"type\": \"module\" }"),
        file("a.js", "require;\n"),
        file("b.cjs", "require;\n"),
        file("c.mts", "require;\n"),
    ]);
    for (path, commonjs) in [("a.js", false), ("b.cjs", true), ("c.mts", false)] {
        assert_eq!(
            read("package-type", typed.clone(), path),
            commonjs,
            "{path}"
        );
        assert_eq!(read("file", typed.clone(), path), commonjs, "{path}");
    }
    // `when.sourceType` gates on the module system the query reads.
    let gated = |module_system: &str| {
        let rule = typescript(json!({
            "languageOptions": { "moduleSystem": module_system },
            "when": { "sourceType": "commonjs" },
            "rule": { "kind": "program" },
        }));
        let snapshot = json!({
            "facts": [], "files": [manifest("{}"), file("src/a.ts", esm)],
            "format": "rerail.repository-snapshot/v1", "revision": 0,
        });
        let report: Value = serde_json::from_str(
            &rule
                .evaluate(snapshot.to_string(), "report".to_owned())
                .expect("evaluation succeeds"),
        )
        .expect("evaluation is JSON");
        report["violations"].as_array().map_or(0, Vec::len)
    };
    assert_eq!(gated("file"), 0);
    assert_eq!(gated("package-type"), 1);
}

#[test]
fn module_system_is_refused_beside_a_source_type_and_for_other_values() {
    for (options, expected) in [
        (
            json!({ "moduleSystem": "package-type", "sourceType": "commonjs" }),
            "moduleSystem decides a file's module system only where sourceType does not",
        ),
        (
            json!({ "moduleSystem": "node" }),
            "moduleSystem must be \"file\" or \"package-type\"",
        ),
    ] {
        let error = compile(
            "typescript",
            json!({ "languageOptions": options, "rule": { "kind": "identifier" } }),
        )
        .err()
        .unwrap_or_else(|| panic!("{options} compiles"));
        assert!(
            error.reason.contains(expected),
            "{options}: {}",
            error.reason
        );
    }
}

#[test]
fn malformed_package_and_jsx_runtime_conditions_are_rejected() {
    let rule = json!({ "kind": "program" });
    for (language, file, expected) in [
        (
            "typescript",
            json!({ "package": {} }),
            "must set `bin`, `nodeVersion`, `dependsOn`, `path` or `serverGraph`",
        ),
        (
            "typescript",
            json!({ "package": { "nodeVersion": { "subsetOf": ">=1.2.3.4" } } }),
            "subsetOf must be an npm semver range",
        ),
        (
            "typescript",
            json!({ "package": { "dependsOn": [] } }),
            "dependsOn must be a non-empty list of package names",
        ),
        (
            "typescript",
            json!({ "package": { "bin": "yes" } }),
            "bin must be true or false",
        ),
        (
            "typescript",
            json!({ "package": { "regex": "x" } }),
            "typescript's package reads the nearest package.json",
        ),
        (
            "typescript",
            json!({ "jsxRuntime": ["preact"] }),
            "jsxRuntime must be a non-empty list",
        ),
        (
            "python",
            json!({ "jsxRuntime": ["react"] }),
            "JSX runtimes are TypeScript's; python has none",
        ),
        (
            "python",
            json!({ "package": { "bin": true } }),
            "python's package is a regex or notRegex over the top package's name",
        ),
    ] {
        let error = compile(language, json!({ "file": file, "rule": rule }))
            .err()
            .unwrap_or_else(|| panic!("{file} compiles"));
        assert!(error.reason.contains(expected), "{file}: {}", error.reason);
    }
}
