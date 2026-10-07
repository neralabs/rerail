use super::*;
use ast_grep_language::LanguageExt;

#[derive(Debug, PartialEq)]
enum Is {
    /// A reference no file-local binding shadows.
    Global,
    /// A reference to a file-local binding that is not an import.
    Local,
    /// A reference to an import binding with this qualified name ("" for a relative import).
    Import(&'static str),
    /// Not in a reference position.
    NotRef,
    /// A reference to a name an unconditional Python `del` unbound.
    Deleted,
}
use Is::{Deleted, Global, Local, NotRef};

fn table(language: GrammarDialect, source: &str) -> ScopeTable {
    let root = language.ast_grep(source);
    ScopeTable::build(root.root(), language, usize::MAX, &|| false).expect("table builds")
}

/// The byte range of the `nth` whole-word occurrence of `name` in `source`.
fn occurrence(source: &str, name: &str, nth: usize) -> Range<usize> {
    occurrences(source, name)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of `{name}` in {source:?}"))
}

/// The byte ranges of every whole-word occurrence of `name` in `source`.
fn occurrences<'a>(source: &'a str, name: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    source
        .match_indices(name)
        .filter(move |(start, _)| {
            let before = source[..*start].chars().next_back();
            let after = source[start + name.len()..].chars().next();
            !before.is_some_and(word) && !after.is_some_and(word)
        })
        .map(move |(start, _)| start..start + name.len())
}

fn classify(table: &ScopeTable, source: &str, name: &str, nth: usize) -> Is {
    let range = occurrence(source, name, nth);
    let Some(reference) = table.reference(&range) else {
        return NotRef;
    };
    match table.resolve(reference, name) {
        None => Global,
        Some(Binding {
            import: Some(Import::Qualified(qualified)),
            ..
        }) => Is::Import(Box::leak(qualified.clone())),
        Some(Binding {
            import: Some(Import::Relative { .. }),
            ..
        }) => Is::Import(""),
        Some(binding) if !binding.shadows() => Global,
        Some(binding) if binding.is_deletion() => Deleted,
        Some(_) => Local,
    }
}

fn check(language: GrammarDialect, source: &str, expected: &[(&str, usize, Is)]) {
    let table = table(language, source);
    for (name, nth, is) in expected {
        assert_eq!(
            &classify(&table, source, name, *nth),
            is,
            "occurrence {nth} of `{name}` in:\n{source}"
        );
    }
}

fn python(source: &str, expected: &[(&str, usize, Is)]) {
    check(GrammarDialect::Python, source, expected);
}

fn typescript(source: &str, expected: &[(&str, usize, Is)]) {
    check(GrammarDialect::TypeScript, source, expected);
    check(GrammarDialect::Tsx, source, expected);
}

#[test]
fn python_module_level_sees_only_earlier_bindings() {
    python(
        "print(len)\nlen = 1\nprint(len)\n",
        &[
            ("print", 0, Global),
            ("len", 0, Global),
            ("len", 1, NotRef),
            ("len", 2, Local),
        ],
    );
    // B911: a name used before its import is still the builtin.
    python(
        "batched(x)\nfrom itertools import batched\nbatched(y)\n",
        &[
            ("batched", 0, Global),
            ("batched", 1, NotRef),
            ("batched", 2, Is::Import("itertools.batched")),
        ],
    );
    // The value of an assignment is read before its target binds.
    python(
        "x = x + 1\nfor y in y: pass\n",
        &[("x", 1, Global), ("y", 1, Global)],
    );
}

#[test]
fn python_function_bodies_see_the_whole_module() {
    python(
        "def f():\n    return len(x)\nlen = 1\nx = 2\n",
        &[("len", 0, Local), ("x", 0, Local)],
    );
    // Inside the body itself, bindings are ordered.
    python(
        "def f():\n    print(1)\n    print = 2\n    print(3)\n",
        &[("print", 0, Global), ("print", 2, Local)],
    );
    // A lambda is deferred too; its defaults are not.
    python(
        "g = lambda: open\nh = lambda a=open: a\nopen = 1\n",
        &[("open", 0, Local), ("open", 1, Global), ("a", 1, Local)],
    );
}

#[test]
fn python_parameters_and_local_definitions_shadow() {
    python(
        "def f(hasattr, *args, key=None, **kw):\n    hasattr(args, key, kw)\n",
        &[
            ("hasattr", 1, Local),
            ("args", 1, Local),
            ("key", 1, Local),
            ("kw", 1, Local),
        ],
    );
    // A local `def hasattr` shadows the builtin for later module code and every function body.
    python(
        "hasattr(a, b)\ndef hasattr(a, b): pass\nhasattr(c, d)\ndef g():\n    hasattr(e, f)\n",
        &[
            ("hasattr", 0, Global),
            ("hasattr", 2, Local),
            ("hasattr", 3, Local),
        ],
    );
    python(
        "Exception = ValueError\ntry:\n    pass\nexcept Exception:\n    pass\n",
        &[("ValueError", 0, Global), ("Exception", 1, Local)],
    );
    // Defaults and annotations run in the enclosing scope.
    python(
        "def f(x: int = x, int=int): pass\n",
        &[("x", 1, Global), ("int", 0, Global), ("int", 2, Global)],
    );
}

#[test]
fn python_class_scope_is_invisible_to_nested_scopes() {
    python(
        "class A:\n    len = 1\n    y = len\n    def m(self):\n        return len\n    z = [len for _ in range(3)]\n",
        &[("len", 1, Local), ("len", 2, Global), ("len", 3, Global)],
    );
    // A comprehension's first iterable runs in the class scope.
    python(
        "class A:\n    xs = [1]\n    ys = [x for x in xs]\n",
        &[("xs", 1, Local), ("x", 0, Local)],
    );
}

#[test]
fn python_global_and_nonlocal_redirect_bindings() {
    // Ruff binds a `global` name only in the declaring scope; other functions and the module
    // still see the builtin (Ruff 0.16.9, ASYNC230 on `open`).
    python(
        "open(w)\ndef f():\n    global open\n    open = 1\n\ndef g():\n    return open\nopen(x)\n",
        &[
            ("open", 0, Global),
            ("open", 1, NotRef),
            ("open", 2, NotRef),
            ("open", 3, Global),
            ("open", 4, Global),
        ],
    );
    python(
        "def f():\n    def h():\n        global exit\n        exit = 1\nexit()\n",
        &[("exit", 2, Global)],
    );
    python(
        "def outer():\n    def inner():\n        nonlocal len\n        len = 1\n    return len\n",
        &[("len", 2, Global)],
    );
    python(
        "def outer():\n    len = 0\n    def inner():\n        nonlocal len\n        return len\n",
        &[("len", 2, Local)],
    );
    // A `global` name read in the function resolves in the module.
    python(
        "x = 1\ndef f():\n    global x\n    return x\n",
        &[("x", 2, Local)],
    );
}

#[test]
fn python_global_statements_leave_other_scopes_their_bindings() {
    // Ruff 0.16.9 (ASYNC251): `global time` binds only in `g`. Its reads follow the module's
    // import until `g` assigns; `a`, `b`, the module and a class body keep the import.
    python(
        "import time\nasync def a():\n    time.sleep(1)\nasync def g():\n    global time\n    time.sleep(0)\n    time = 3\n    time.sleep(1)\nasync def b():\n    time.sleep(1)\ntime.sleep(2)\n",
        &[
            ("time", 1, Is::Import("time")),
            ("time", 2, NotRef),
            ("time", 3, Is::Import("time")),
            ("time", 5, Local),
            ("time", 6, Is::Import("time")),
            ("time", 7, Is::Import("time")),
        ],
    );
    python(
        "import time\ndef f():\n    global time\n    time = 3\n    async def h():\n        time.sleep(1)\ndef f2():\n    global time\n    async def h2():\n        time.sleep(1)\nclass C:\n    global time\n    time = 3\nasync def b():\n    time.sleep(1)\n",
        &[
            ("time", 3, Local),
            ("time", 5, Is::Import("time")),
            ("time", 8, Is::Import("time")),
        ],
    );
    // An import after the declaring `def` still resolves for the module and every function.
    python(
        "def f():\n    global time\nx = time\nimport time\nasync def g():\n    time.sleep(1)\n",
        &[("time", 1, Global), ("time", 3, Is::Import("time"))],
    );
}

#[test]
fn python_global_statements_leave_builtins_to_other_scopes() {
    // Ruff 0.16.9 (ASYNC250, PIE808): only the declaring function stops seeing the builtin
    // (`has_builtin_binding` finds its `global` binding); the rest of the file keeps it.
    python(
        "async def a():\n    input()\nasync def k():\n    global input\n    input()\nasync def m():\n    global input\n    input = 3\n    input()\nasync def b():\n    input()\ninput()\n",
        &[
            ("input", 0, Global),
            ("input", 1, NotRef),
            ("input", 2, Local),
            ("input", 5, Local),
            ("input", 6, Global),
            ("input", 7, Global),
        ],
    );
    python(
        "def g():\n    global set\n    set = 1\nx == set\ndef h():\n    return x == set\n",
        &[("set", 2, Global), ("set", 3, Global)],
    );
    python(
        "import numpy\ndef g():\n    global numpy\n    numpy = None\nnumpy.dtype(x) == int\n",
        &[("numpy", 3, Is::Import("numpy"))],
    );
}

#[test]
fn python_global_declarations_resolve_undefined_module_names_like_ruff() {
    // F821 is silent on a module-level use after a function or class declares the name `global`,
    // assigned or not, but for `resolves` that placeholder shadows nothing.
    let source = "def f():\n    global undefined_b\nundefined_b\nclass C:\n    global undefined_k\nundefined_k\n";
    let table = table(GrammarDialect::Python, source);
    for name in ["undefined_b", "undefined_k"] {
        let range = occurrence(source, name, 1);
        let reference = table.reference(&range).expect("reference");
        let binding = table.resolve(reference, name).expect("resolved for F821");
        assert!(!binding.shadows(), "`{name}` shadows nothing");
    }
}

#[test]
fn python_except_names_are_unbound_after_their_handler() {
    // Ruff 0.16.9 (ASYNC250, ASYNC251, F821): after the handler the name reads as it did before
    // it, in the module and in every function body.
    python(
        "import time\ntry:\n    pass\nexcept Exception as time:\n    time.sleep(0)\ntime.sleep(1)\nasync def a():\n    time.sleep(2)\n    try:\n        pass\n    except Exception as input:\n        input()\n    input()\n    def inner():\n        try:\n            pass\n        except* Exception as time:\n            pass\n        async def c():\n            time.sleep(3)\ntry:\n    pass\nexcept Exception as frozenset:\n    frozenset\nx == frozenset\ndef f():\n    return frozenset\n",
        &[
            ("time", 2, Local),
            ("time", 3, Is::Import("time")),
            ("time", 4, Is::Import("time")),
            ("input", 1, Local),
            ("input", 2, Global),
            ("time", 6, Is::Import("time")),
            ("frozenset", 1, Local),
            ("frozenset", 2, Global),
            ("frozenset", 3, Global),
        ],
    );
}

#[test]
fn python_del_unbinds_unconditionally_and_reads_its_own_scope() {
    // Ruff 0.16.9's `Deletion`: an unconditional `del` unbinds the name for later reads, in the
    // scope and in function bodies read after the module; the `del` itself reads the scope's own
    // binding only.
    python(
        "x = 1\ndel x\nprint(x)\ndef f():\n    return x\n",
        &[("x", 1, Local), ("x", 2, Deleted), ("x", 3, Deleted)],
    );
    python(
        "len = 1\ndel len\nlen(y)\n",
        &[("len", 1, Local), ("len", 2, Deleted)],
    );
    python("x = 1\ndef f():\n    del x\n", &[("x", 1, Global)]);
    python(
        "def f():\n    x = 1\n    del x, y\n    x\n",
        &[("x", 1, Local), ("y", 0, Global), ("x", 2, Deleted)],
    );
    // A new binding after the `del` binds again.
    python("x = 1\ndel x\nx = 2\nprint(x)\n", &[("x", 3, Local)]);
    // Conditional deletions leave the binding: branches, loops' and `try`'s `else`, handlers.
    for source in [
        "x = 1\nif c:\n    del x\nprint(x)\n",
        "x = 1\nwhile c:\n    del x\nprint(x)\n",
        "x = 1\nfor i in y:\n    pass\nelse:\n    del x\nprint(x)\n",
        "x = 1\ntry:\n    pass\nexcept E:\n    del x\nprint(x)\n",
        "x = 1\ntry:\n    pass\nexcept E:\n    pass\nelse:\n    del x\nprint(x)\n",
        "x = 1\nmatch c:\n    case 1:\n        del x\nprint(x)\n",
        "x = 1\nif c:\n    def f():\n        del x\n",
    ] {
        let table = table(GrammarDialect::Python, source);
        assert!(
            !table.scopes.iter().any(|scope| scope
                .bindings
                .values()
                .flatten()
                .any(Binding::is_deletion)),
            "no deletion in:\n{source}"
        );
    }
    // A `for` or `try` body and a `with` body are not conditional; a handler of an enclosing
    // function does not make a nested function's `del` conditional.
    python(
        "x = 1\nfor i in y:\n    del x\nprint(x)\n",
        &[("x", 2, Deleted)],
    );
    python(
        "x = 1\ntry:\n    del x\nfinally:\n    pass\nprint(x)\n",
        &[("x", 2, Deleted)],
    );
    python(
        "try:\n    pass\nexcept E:\n    def f():\n        x = 1\n        del x\n        x\n",
        &[("x", 2, Deleted)],
    );
}

#[test]
fn python_bindings_carry_names_branches_and_import_flags() {
    let source = "import os.path\nimport a as b\nimport q as q\nfrom m import x\nfrom m import y as z\nfrom m import w as w\nfrom n import *\nif c:\n    import k\nelif d:\n    import k\nelse:\n    k = 1\ntry:\n    t = 1\nexcept E:\n    t = 2\nelse:\n    t = 3\nfinally:\n    t = 4\nwith f() as u:\n    u = 1\nmatch v:\n    case [g]:\n        g = 1\ndef h():\n    global G\n    G = 1\n    def inner():\n        nonlocal s\n        s = 2\n    s = 1\n";
    let table = table(GrammarDialect::Python, source);
    let find = |scope: usize, name: &str, index: usize| -> &Binding {
        &table.scopes[scope].bindings[name][index]
    };
    let text = |binding: &Binding| &source[binding.name.expect("a name").range()];
    assert_eq!(text(find(0, "os", 0)), "os.path");
    assert_eq!(find(0, "os", 0).submodule.as_deref(), Some("os.path"));
    assert_eq!(text(find(0, "b", 0)), "b");
    assert!(!find(0, "b", 0).explicit_export && find(0, "q", 0).explicit_export);
    assert_eq!(text(find(0, "x", 0)), "x");
    assert_eq!(text(find(0, "z", 0)), "z");
    assert!(find(0, "w", 0).explicit_export && find(0, "w", 0).submodule.is_none());
    assert_eq!(table.scopes[0].stars.len(), 1);
    // Each `if`/`elif`/`else` body is its own branch; a `try` body and its `else` share one.
    let [k1, k2, k3] = [0, 1, 2].map(|index| find(0, "k", index).branch);
    assert!(k1 != 0 && k1 != k2 && k2 != k3 && k1 != k3);
    let [t1, t2, t3, t4] = [0, 1, 2, 3].map(|index| find(0, "t", index).branch);
    assert!(t1 == t3 && t1 != t2 && t1 != t4 && t2 != t4);
    // A `with` item binds outside the body's branch; a `case` capture outside its body's.
    let [u1, u2] = [0, 1].map(|index| find(0, "u", index).branch);
    assert!(u1 == 0 && u2 != 0);
    let [g1, g2] = [0, 1].map(|index| find(0, "g", index).branch);
    assert!(g1 == 0 && g2 != 0);
    assert_eq!(find(0, "os", 0).branch, 0);
    // `global` and `nonlocal` bindings are outer; the enclosing function's own is not.
    let function = table
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Function)
        .expect("h");
    let globals = &table.scopes[function].bindings["G"];
    assert!(globals.iter().all(|binding| binding.outer) && globals.len() == 2);
    let s = &table.scopes[function].bindings["s"];
    assert_eq!(
        s.iter().map(|binding| binding.outer).collect::<Vec<_>>(),
        [false, true],
        "the function's own `s = 1`, then `inner`'s after the body"
    );
}

#[test]
fn python_nonlocal_names_read_the_enclosing_binding_but_are_not_references() {
    let source = "def outer():\n    x = 1\n    def inner():\n        nonlocal x\n        x = 2\n";
    let table = table(GrammarDialect::Python, source);
    let range = occurrence(source, "x", 1);
    assert!(table.reference(&range).is_none());
    let reference = table
        .references
        .iter()
        .find(|reference| reference.start == range.start)
        .expect("a nonlocal read");
    assert_eq!(reference.access, Access::Nonlocal);
    let binding = table.resolve(reference, "x").expect("resolved");
    assert_eq!(binding.offset, occurrence(source, "1", 0).end);
}

#[test]
fn python_walrus_binds_outside_its_comprehension() {
    python(
        "def f():\n    [y := n for n in range(3)]\n    return y\n",
        &[("n", 0, Local), ("y", 1, Local)],
    );
    python(
        "[x for x in range(3)]\nprint(x)\n",
        &[("x", 0, Local), ("x", 2, Global)],
    );
}

#[test]
fn python_every_binding_form_shadows() {
    let source = "\
import os
import os.path as osp
import a.b.c
from m import (p, q as r)
from . import rel
from .pkg import rel2
from star import *
with ctx() as (w1, w2): pass
try:
    pass
except E as exc:
    print(exc)
for f1, (f2, *f3) in z: pass
a1, [a2, *a3] = z
an: int = 1
ann: int
aug += 1
class K[T](Base): pass
def fn[U, *Us, **Ps](): pass
type Alias[V] = list[V]
match z:
    case Point(x=m1, y=[m2, *m3]) | {\"k\": m4, **m5} as m6:
        pass
    case Color.RED:
        pass
    case _:
        pass
lam = lambda l1, l2=1: l1
print(os, osp, a, p, r, rel, rel2, w1, w2, f1, f2, f3, a1, a2, a3, an, ann, aug, K, fn, Alias, m1, m2, m3, m4, m5, m6, lam, starred, Color)
";
    let table = table(GrammarDialect::Python, source);
    let last = |name: &str| {
        let count = source
            .match_indices(name)
            .filter(|(start, _)| {
                let word = |c: char| c.is_alphanumeric() || c == '_';
                !source[..*start].chars().next_back().is_some_and(word)
                    && !source[start + name.len()..]
                        .chars()
                        .next()
                        .is_some_and(word)
            })
            .count();
        classify(&table, source, name, count - 1)
    };
    assert_eq!(last("os"), Is::Import("os"));
    assert_eq!(last("osp"), Is::Import("os.path"));
    assert_eq!(last("a"), Is::Import("a"));
    assert_eq!(last("p"), Is::Import("m.p"));
    assert_eq!(last("r"), Is::Import("m.q"));
    assert_eq!(last("rel"), Is::Import(""));
    assert_eq!(last("rel2"), Is::Import(""));
    for name in [
        "w1", "w2", "exc", "f1", "f2", "f3", "a1", "a2", "a3", "an", "aug", "K", "fn", "Alias",
        "m1", "m2", "m3", "m4", "m5", "m6", "lam",
    ] {
        assert_eq!(last(name), Local, "{name}");
    }
    // An annotation without a value, a star import and a value pattern bind nothing.
    assert_eq!(last("ann"), Global);
    assert_eq!(last("starred"), Global);
    assert_eq!(last("Color"), Global);
    // Type parameters are visible in their own declaration only.
    python(
        "def f[T](x: T) -> T:\n    return T\nclass C[S](list[S]): pass\ntype A[V] = V\nprint(T, S, V)\n",
        &[
            ("T", 1, Local),
            ("T", 2, Local),
            ("T", 3, Local),
            ("S", 1, Local),
            ("V", 1, Local),
            ("T", 4, Global),
            ("S", 2, Global),
            ("V", 2, Global),
        ],
    );
}

#[test]
fn python_reference_positions() {
    python(
        "obj.attr\nf(key=1)\nimport mod\nfrom pkg import name\nglobal g\n",
        &[
            ("obj", 0, Global),
            ("attr", 0, NotRef),
            ("key", 0, NotRef),
            ("mod", 0, NotRef),
            ("pkg", 0, NotRef),
            ("name", 0, NotRef),
            ("g", 0, NotRef),
        ],
    );
    // Augmented assignment reads its target before binding it.
    python("x += 1\n", &[("x", 0, Global)]);
    // f-string interpolations and decorators are loads.
    python(
        "@cache\ndef f(): return f\"{open}\"\n",
        &[("cache", 0, Global), ("open", 0, Global), ("f", 0, NotRef)],
    );
}

#[test]
fn python_function_scoped_imports() {
    python(
        "def f():\n    import subprocess\n    subprocess.run()\nsubprocess.run()\n",
        &[
            ("subprocess", 1, Is::Import("subprocess")),
            ("subprocess", 2, Global),
        ],
    );
    python("import os as o\no.path\n", &[("o", 1, Is::Import("os"))]);
    // A later non-import binding replaces the import.
    python("import os\nos = None\nos.path\n", &[("os", 2, Local)]);
}

#[test]
fn python_reads_in_evaluation_order() {
    // A comprehension's element runs after its conditions; a conditional expression's body after
    // its test. Both see what a walrus there binds.
    python(
        "def f(blocks):\n    return [text for b in blocks if (text := b.text)]\n",
        &[("text", 0, Local)],
    );
    python(
        "v = named if (named := g()) else other\n",
        &[("named", 0, Local), ("other", 0, Global)],
    );
}

#[test]
fn python_future_annotations_are_read_after_the_module() {
    let source = "def f(x: Later) -> Later: ...\nclass Later: ...\n";
    python(source, &[("Later", 0, Global), ("Later", 1, Global)]);
    python(
        &format!("from __future__ import annotations\n{source}"),
        &[("Later", 0, Local), ("Later", 1, Local)],
    );
}

#[test]
fn typescript_var_hoists_and_let_is_block_scoped() {
    typescript(
        "alert(x);\nfunction f() {\n  if (a) { var alert = 1; let confirm = 2; }\n  alert(); confirm();\n}\nalert();\n",
        &[
            ("alert", 0, Global),
            ("alert", 2, Local),
            ("confirm", 1, Global),
            ("alert", 3, Global),
        ],
    );
    // No ordering: a later declaration shadows an earlier use (hoisting and the TDZ alike).
    typescript("print();\nconst print = 1;\n", &[("print", 0, Local)]);
    // Function declarations bind in their block (eslint-scope, ES2015+).
    typescript("{ function name() {} }\nname();\n", &[("name", 1, Global)]);
}

#[test]
fn typescript_blocks_are_scopes_only_from_es2015() {
    // eslint-scope nests block and switch scopes only for ecmaVersion 6 and later; a catch
    // clause is a scope either way.
    let source = "\
{ function a() {} }
a;
switch (x) { case 1: function b() {} }
b;
function f() { if (x) { function c() {} } c; }
c;
try {} catch (d) { d; }
d;
";
    let build = |block_scopes: bool| {
        let root = GrammarDialect::TypeScript.ast_grep(source);
        let reading = Reading {
            block_scopes,
            ..Reading::default()
        };
        ScopeTable::build_with(
            root.root(),
            GrammarDialect::TypeScript,
            usize::MAX,
            &|| false,
            reading,
        )
        .expect("table builds")
    };
    let es5 = build(false);
    let es2015 = build(true);
    for (name, nth, before, after) in [
        ("a", 1, Local, Global),
        ("b", 1, Local, Global),
        ("c", 1, Local, Global),
        ("c", 2, Global, Global),
        ("d", 1, Local, Local),
        ("d", 2, Global, Global),
    ] {
        assert_eq!(
            classify(&es5, source, name, nth),
            before,
            "ES5 `{name}` {nth}"
        );
        assert_eq!(
            classify(&es2015, source, name, nth),
            after,
            "ES2015 `{name}` {nth}"
        );
    }
    assert!(Reading::default().block_scopes);
}

#[test]
fn typescript_parameters_patterns_and_catch() {
    typescript(
        "function f(a, { b, c: [d, ...e] }, g = h, ...i) { a; b; d; e; g; i; c; h; }\n",
        &[
            ("a", 1, Local),
            ("b", 1, Local),
            ("d", 1, Local),
            ("e", 1, Local),
            ("g", 1, Local),
            ("i", 1, Local),
            ("c", 1, Global),
            ("h", 0, Global),
        ],
    );
    typescript(
        "try {} catch ({ message }) { message; }\nmessage;\ntry {} catch (err) { err; }\n",
        &[
            ("message", 1, Local),
            ("message", 2, Global),
            ("err", 1, Local),
        ],
    );
    typescript(
        "const f = (x) => x;\nconst g = y => y;\nx; y;\n",
        &[
            ("x", 1, Local),
            ("y", 1, Local),
            ("x", 2, Global),
            ("y", 2, Global),
        ],
    );
    // Destructuring assignment (not a declaration) writes references.
    typescript(
        "[name, top] = pair;\n({ status } = res);\n",
        &[
            ("name", 0, Global),
            ("top", 0, Global),
            ("status", 0, Global),
        ],
    );
}

#[test]
fn typescript_function_and_class_expression_names_and_arguments() {
    typescript(
        "const f = function self() { self; arguments; };\nself;\nconst C = class Me { m() { Me; } };\nMe;\nconst a = () => arguments;\n",
        &[
            ("self", 1, Local),
            ("arguments", 0, Local),
            ("self", 2, Global),
            ("Me", 1, Local),
            ("Me", 2, Global),
            ("arguments", 1, Global),
        ],
    );
}

#[test]
fn typescript_value_and_type_namespaces_are_separate() {
    typescript(
        "interface Window { x: number }\nlet w: Window = window;\nconst Event = 1;\nlet e: Event;\nclass Node {}\nlet n: Node = new Node();\n",
        &[
            ("Window", 1, Local),
            ("window", 0, Global),
            ("Event", 1, Global),
            ("Node", 1, Local),
            ("Node", 2, Local),
        ],
    );
    typescript(
        "type T<U> = U extends Array<infer R> ? R : never;\nlet r: R; let u: U;\nfunction g<V>(v: V): V { return v; }\nlet v: V;\n",
        &[
            ("U", 1, Local),
            ("R", 1, Local),
            ("R", 2, Global),
            ("U", 2, Global),
            ("V", 1, Local),
            ("V", 2, Local),
            ("V", 3, Global),
        ],
    );
    // Declaration merging: every declaration binds its own spaces.
    typescript(
        "enum E { A, B = A }\nnamespace E { export const z = 1; }\nlet e: E = E.A;\nimport * as ns from \"m\";\nlet x: ns.T = ns.v;\n",
        &[
            ("A", 1, Local),
            ("E", 2, Local),
            ("E", 3, Local),
            ("ns", 1, Local),
            ("ns", 2, Local),
        ],
    );
}

#[test]
fn typescript_declare_global_does_not_shadow() {
    typescript(
        "declare global { interface Window { foo: string } var gg: number; }\nlet w: Window = window;\ngg;\n",
        &[("Window", 1, Global), ("gg", 1, Global)],
    );
    typescript(
        "declare module \"m\" { const inner: number; }\ninner;\ndeclare const outer: number;\nouter;\n",
        &[("inner", 1, Global), ("outer", 1, Local)],
    );
}

#[test]
fn typescript_imports_exports_and_properties() {
    typescript(
        "import D, { a as b, c } from \"m\";\nimport * as ns from \"n\";\nimport eq = require(\"x\");\nD; a; b; c; ns; eq;\nexport { b as out, missing };\nexport { gone } from \"m\";\n",
        &[
            ("D", 1, Local),
            ("a", 1, Global),
            ("b", 1, Local),
            ("c", 1, Local),
            ("ns", 1, Local),
            ("eq", 1, Local),
            ("b", 2, Local),
            ("out", 0, NotRef),
            ("missing", 0, Global),
            ("gone", 0, NotRef),
        ],
    );
    typescript(
        "obj.alert;\nconst o = { alert, confirm: 1, [key]: 2 };\nlbl: for (;;) { break lbl; }\n",
        &[
            ("obj", 0, Global),
            ("alert", 0, NotRef),
            ("alert", 1, Global),
            ("confirm", 0, NotRef),
            ("key", 0, Global),
            ("lbl", 0, NotRef),
        ],
    );
    typescript(
        "for (const k in o) { k; }\nfor (k2 in o) {}\nfor (var v of xs) {}\nv;\n",
        &[("k", 1, Local), ("k2", 0, Global), ("v", 1, Local)],
    );
}

#[test]
fn typescript_signatures_keep_their_parameters() {
    typescript(
        "interface I { m(alert: number): void; (confirm: string): void; [key: string]: any }\ntype F = (prompt: number) => void;\nalert; confirm; key; prompt;\n",
        &[
            ("alert", 1, Global),
            ("confirm", 1, Global),
            ("key", 1, Global),
            ("prompt", 1, Global),
        ],
    );
    typescript(
        "let m: { [P in keyof T]: P };\nlet p: P;\n",
        &[("P", 1, Local), ("P", 2, Global)],
    );
}

#[test]
fn tsx_references_capitalized_jsx_tag_names() {
    check(
        GrammarDialect::Tsx,
        "import Foo from \"f\";\nconst v = <Foo.Bar a={alert}><div /><Baz></Baz><Foo /></Foo.Bar>;\n",
        &[
            ("Foo", 1, Local),
            ("alert", 0, Global),
            ("div", 0, NotRef),
            ("Baz", 0, Global),
            ("Baz", 1, Global),
            ("Foo", 2, Local),
            ("a", 0, NotRef),
        ],
    );
}

#[test]
fn eslint_global_directives_are_collected_from_block_comments_only() {
    let source = "/* global foo, bar: writable, baz:off -- why */\n// global line\nconst s = \"/* global str */\";\n/*globals  qux */\n/* globalx no */\nfoo;\n";
    let table = table(GrammarDialect::TypeScript, source);
    assert_eq!(
        table.directives.declared.iter().collect::<Vec<_>>(),
        ["bar", "foo", "qux"]
    );
    assert_eq!(table.directives.off.iter().collect::<Vec<_>>(), ["baz"]);
}

/// What the last whole-word occurrence of `name` binds to: its form, the texts of its site,
/// value and annotation (the value with TypeScript's write rule applied), and its writes and
/// property mutations; `None` for a global or builtin.
#[derive(Debug, PartialEq)]
struct Bound {
    form: Form,
    site: Option<&'static str>,
    value: Option<&'static str>,
    annotation: Option<&'static str>,
    writes: u32,
    mutated: bool,
}

fn bound(table: &ScopeTable, source: &'static str, name: &str) -> Option<Bound> {
    let ranges = occurrences(source, name).collect::<Vec<_>>();
    let reference = ranges
        .iter()
        .rev()
        .find_map(|range| table.reference(range))
        .unwrap_or_else(|| panic!("a reference to `{name}` in {source:?}"));
    let (binding, value) = table.binding(reference, name)?;
    let root = table.language.ast_grep(source);
    let text = |node: Option<NodeRef>| {
        node.map(|node| {
            let found = node.find(root.root()).expect("the node is in the tree");
            assert_eq!(found.range(), node.range());
            &source[node.range()]
        })
    };
    Some(Bound {
        form: binding.site.form,
        site: text(binding.site.node),
        value: text(value),
        annotation: text(binding.site.annotation),
        writes: binding.writes,
        mutated: binding.mutated,
    })
}

/// A name, and its binding's form and the texts of its site, value and annotation.
type ExpectedSite<'a> = (
    &'a str,
    Option<(Form, &'a str, Option<&'a str>, Option<&'a str>)>,
);

fn sites(language: GrammarDialect, source: &'static str, expected: &[ExpectedSite<'_>]) {
    let table = table(language, source);
    for (name, expected) in expected {
        let found = bound(&table, source, name);
        let found = found.as_ref().map(|found| {
            (
                found.form,
                found.site.unwrap_or(""),
                found.value,
                found.annotation,
            )
        });
        assert_eq!(found, *expected, "`{name}` in:\n{source}");
    }
}

#[test]
fn python_bindings_carry_their_site_value_and_annotation() {
    let source = "import k.l\nfrom m import n as o\n\
        x = 1\ny: dict = {}\na = b = []\n(c) = 2\nd, e = 1, 2\nf = 0\nf += 1\n\
        if (g := 3): pass\nfor h in []: pass\nwith open() as i: pass\n\
        try: pass\nexcept E as j: print(j)\n\
        def p(q, r: int = 1, *s: str, **t): return q, r, s, t\n\
        class U: pass\nw = [v for v in []]\n\
        match z:\n    case [cap, *rest]: print(cap, rest)\n\
        def glob():\n    global x\n    return x\n\
        print(x, y, a, b, c, d, f, g, h, i, k, o, p, U, len)\n";
    use Form::*;
    sites(
        GrammarDialect::Python,
        source,
        &[
            ("x", Some((Assignment, "x = 1", Some("1"), None))),
            (
                "y",
                Some((Annotated, "y: dict = {}", Some("{}"), Some("dict"))),
            ),
            ("a", Some((Assignment, "a = b = []", Some("[]"), None))),
            ("b", Some((Assignment, "b = []", Some("[]"), None))),
            ("c", Some((Assignment, "(c) = 2", Some("2"), None))),
            ("d", Some((Assignment, "d, e = 1, 2", Some("1"), None))),
            ("f", Some((Augmented, "f += 1", None, None))),
            ("g", Some((Named, "g := 3", Some("3"), None))),
            ("h", Some((For, "for h in []: pass", None, None))),
            ("i", Some((With, "open() as i", None, None))),
            ("j", Some((Except, "E as j", None, None))),
            ("k", Some((Import, "k.l", None, None))),
            ("o", Some((Import, "n as o", None, None))),
            ("q", Some((Parameter, "q", None, None))),
            ("r", Some((Parameter, "r: int = 1", None, Some("int")))),
            ("s", Some((Parameter, "*s: str", None, Some("str")))),
            ("t", Some((Parameter, "**t", None, None))),
            (
                "p",
                Some((
                    Definition,
                    "def p(q, r: int = 1, *s: str, **t): return q, r, s, t",
                    None,
                    None,
                )),
            ),
            ("U", Some((Definition, "class U: pass", None, None))),
            ("v", Some((For, "for v in []", None, None))),
            ("cap", Some((Match, "cap", None, None))),
            ("rest", Some((Match, "*rest", None, None))),
            ("len", None),
        ],
    );
    // A `global` read continues in the module's binding.
    let table = table(GrammarDialect::Python, source);
    let range = occurrence(source, "x", 2);
    let reference = table.reference(&range).expect("a reference");
    let (binding, value) = table.binding(reference, "x").expect("bound");
    assert_eq!(binding.site.form, Form::Assignment);
    assert_eq!(value.map(|value| &source[value.range()]), Some("1"));
    // Ruff's placeholder for an undefined `global` name binds nothing.
    let source = "def f():\n    global late\n    late = 1\nprint(late)\n";
    sites(GrammarDialect::Python, source, &[("late", None)]);
}

/// Ruff's `find_binding_value` pairs unpacked targets with a tuple or list display's elements; the
/// table pairs where the runtime binds that element: equal arity, or elements before a starred one.
#[test]
fn python_unpacking_pairs_targets_with_display_elements() {
    let source = "a, b = 1, 2\n(c, d) = (3, 4)\n[e, f] = [5, 6]\ng, [h, i] = 7, (8, 9)\n\
        j, (k) = 10, (11)\nl, m = 12, 13, 14\nn, *o, p = 15, 16, 17\nq, *r, s = 18, 19, 20, 21\n\
        t, u = *v, 22\nw, x = pair\ny, z = {23, 24}\naa = ab, ac = 25, 26\nad, ae = af, ag = 27, 28\n\
        ((ah), ai) = ((29), 30)\nfor aj, ak in [(1, 2)]: pass\nwith f() as (al, am): pass\n\
        (an, ao) = ((31, 32))\nap, = 33,\naq, ar, as_ = 34, *at, 35\n\
        print(a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, w, x, y, z)\n\
        print(aa, ab, ac, ad, ae, af, ag, ah, ai, aj, ak, al, am, an, ao, ap, aq, ar, as_)\n";
    let value = |name: &str| -> Option<&'static str> {
        let table = table(GrammarDialect::Python, source);
        bound(&table, source, name).and_then(|bound| bound.value)
    };
    let expected: &[(&str, Option<&str>)] = &[
        ("a", Some("1")),
        ("b", Some("2")),
        ("c", Some("3")),
        ("d", Some("4")),
        ("e", Some("5")),
        ("f", Some("6")),
        ("g", Some("7")),
        ("h", Some("8")),
        ("i", Some("9")),
        ("j", Some("10")),
        ("k", Some("11")),
        // Unequal arity raises; nothing is bound.
        ("l", None),
        ("m", None),
        // A starred target: before it always, after it only where arity lines up.
        ("n", Some("15")),
        ("o", None),
        ("p", Some("17")),
        ("q", Some("18")),
        ("r", None),
        ("s", None),
        // A starred value hides every position from its own on.
        ("t", None),
        ("u", None),
        ("w", None),
        ("x", None),
        // A set's order is not its display's.
        ("y", None),
        ("z", None),
        ("aa", Some("25, 26")),
        ("ab", Some("25")),
        ("ac", Some("26")),
        ("ad", Some("27")),
        ("ae", Some("28")),
        ("af", Some("27")),
        ("ag", Some("28")),
        ("ah", Some("29")),
        ("ai", Some("30")),
        ("aj", None),
        ("ak", None),
        ("al", None),
        ("am", None),
        ("an", Some("31")),
        ("ao", Some("32")),
        ("ap", Some("33")),
        ("aq", Some("34")),
        ("ar", None),
        ("as_", None),
    ];
    for (name, expected) in expected {
        assert_eq!(value(name), *expected, "`{name}`");
    }
}

#[test]
fn python_values_follow_the_binding_the_read_sees() {
    let source = "x = 1\nprint(x)\nx = [2]\nprint(x)\ndef f():\n    return x\nx = 3\n";
    let table = table(GrammarDialect::Python, source);
    let value_at = |nth: usize| {
        let reference = table
            .reference(&occurrence(source, "x", nth))
            .expect("a reference");
        let (_, value) = table.binding(reference, "x").expect("bound");
        value.map(|value| &source[value.range()])
    };
    assert_eq!(value_at(1), Some("1"));
    assert_eq!(value_at(3), Some("[2]"));
    // A function body sees the module's last binding.
    assert_eq!(value_at(4), Some("3"));
}

#[test]
fn typescript_bindings_carry_a_value_only_when_effectively_const() {
    let source = "const a = 1;\nlet b = 2; b = 3;\nlet c = 4;\nvar d = 5; var d;\n\
        let [e] = [1];\nconst f: number = 6;\nlet g;\nconst h = [1]; h.push(2);\n\
        const i = {}; i.x = 1;\nlet j = 1; j++;\nlet k = 1; ({ k } = o);\n\
        let l = 1; [l] = o;\nlet m = 1; for (m of o) {}\nlet n = 1; n! += 1;\n\
        const s = { t: 1 }; s.t.u.v = 2; s[0]++;\n\
        function fn(p: string, q = 1) { use(p, q, arguments); }\n\
        import w, { x as y } from 'z';\n\
        use(a, b, c, d, e, f, g, h, i, j, k, l, m, n, s, fn, w, y, undeclared);\n";
    use Form::*;
    let table = table(GrammarDialect::TypeScript, source);
    let expected = [
        ("a", Assignment, "a = 1", Some("1"), None, 0, false),
        ("b", Assignment, "b = 2", None, None, 1, false),
        ("c", Assignment, "c = 4", Some("4"), None, 0, false),
        ("d", Assignment, "d = 5", None, None, 0, false),
        ("e", Assignment, "[e] = [1]", Some("1"), None, 0, false),
        (
            "f",
            Assignment,
            "f: number = 6",
            Some("6"),
            Some("number"),
            0,
            false,
        ),
        ("g", Assignment, "g", None, None, 0, false),
        ("h", Assignment, "h = [1]", Some("[1]"), None, 0, true),
        ("i", Assignment, "i = {}", Some("{}"), None, 0, true),
        ("j", Assignment, "j = 1", None, None, 1, false),
        ("k", Assignment, "k = 1", None, None, 1, false),
        ("l", Assignment, "l = 1", None, None, 1, false),
        ("m", Assignment, "m = 1", None, None, 1, false),
        ("n", Assignment, "n = 1", None, None, 1, false),
        (
            "s",
            Assignment,
            "s = { t: 1 }",
            Some("{ t: 1 }"),
            None,
            0,
            true,
        ),
        ("p", Parameter, "p: string", None, Some("string"), 0, false),
        ("q", Parameter, "q = 1", None, None, 0, false),
        (
            "fn",
            Definition,
            "function fn(p: string, q = 1) { use(p, q, arguments); }",
            None,
            None,
            0,
            false,
        ),
        ("w", Import, "w, { x as y }", None, None, 0, false),
        ("y", Import, "x as y", None, None, 0, false),
    ];
    for (name, form, site, value, annotation, writes, mutated) in expected {
        let found = bound(&table, source, name).unwrap_or_else(|| panic!("`{name}` is bound"));
        assert_eq!(
            found,
            Bound {
                form,
                site: Some(site),
                value,
                annotation,
                writes,
                mutated,
            },
            "`{name}`"
        );
    }
    assert_eq!(bound(&table, source, "undeclared"), None);
    let arguments = bound(&table, source, "arguments").expect("bound");
    assert_eq!((arguments.form, arguments.site), (Implicit, None));
}

#[test]
fn builds_stop_at_their_budget() {
    let source = "x = 1\n".repeat(100);
    let root = GrammarDialect::Python.ast_grep(&source);
    let full = ScopeTable::build(root.root(), GrammarDialect::Python, usize::MAX, &|| false)
        .expect("builds");
    assert!(full.cost > 100);
    assert_eq!(
        ScopeTable::build(root.root(), GrammarDialect::Python, full.cost - 1, &|| {
            false
        })
        .err(),
        Some(BuildError::Budget)
    );
    assert!(ScopeTable::build(root.root(), GrammarDialect::Python, full.cost, &|| false).is_ok());
    let deep = format!("x = {}1{}\n", "(".repeat(5_000), ")".repeat(5_000));
    let root = GrammarDialect::Python.ast_grep(&deep);
    assert_eq!(
        ScopeTable::build(root.root(), GrammarDialect::Python, usize::MAX, &|| false).err(),
        Some(BuildError::Depth)
    );
    let long = "x = 1\n".repeat(1_000);
    let root = GrammarDialect::Python.ast_grep(&long);
    assert_eq!(
        ScopeTable::build(root.root(), GrammarDialect::Python, usize::MAX, &|| true).err(),
        Some(BuildError::Deadline)
    );
}

/// Where Ruff reports a reference: its source range, or for a name in a string type definition
/// the range its string maps it to (exact in a simple string, the whole string otherwise).
fn source_range(table: &ScopeTable, reference: &Reference) -> std::ops::Range<usize> {
    let mut range = reference.start..reference.end;
    while range.start & ANNOTATION_RANGE != 0 {
        let offset = annotation_offset((range.start & !ANNOTATION_RANGE) >> ANNOTATION_SHIFT);
        let Some(mapped) = table
            .annotations
            .iter()
            .find(|annotation| annotation.tree.offset() == offset)
            .and_then(|annotation| annotation.tree.host_span(range.clone()))
        else {
            break;
        };
        range = mapped;
    }
    range
}

/// Every reference the table resolves to no file-local binding, against an offline oracle's list
/// (`rerail-fixtures/python/scope/generate_python.py`, `rerail-fixtures/ecma/scope/generate_typescript.mjs`): Ruff's
/// F821 for Python, typescript-eslint's unresolved references for
/// TypeScript. Names the oracle treats as builtins or ambient globals are left out on both sides.
fn oracle_differences(
    language: GrammarDialect,
    source: &str,
    expected: &serde_json::Value,
) -> Vec<String> {
    let root = language.ast_grep(source);
    if root.root().get_doc().tree.root_node().has_error() {
        return vec!["the source has syntax errors".to_owned()];
    }
    // Ruff defers every annotation for a target of Python 3.14 or later (PEP 649).
    let deferred = expected["targetVersion"]
        .as_str()
        .and_then(|target| target.strip_prefix("py3")?.parse::<u8>().ok())
        .is_some_and(|minor| minor >= 14);
    let reading = Reading {
        deferred_annotations: deferred,
        ..Reading::default()
    };
    let table = ScopeTable::build_with(root.root(), language, usize::MAX, &|| false, reading)
        .expect("table builds");
    let builtins = expected["builtins"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let ours = table
        .references
        .iter()
        .filter(|reference| reference.access != Access::Nonlocal)
        .filter_map(|reference| {
            let name = table.reference_name(reference, source, 0)?;
            // A deleted builtin name is unbound too, as is one a function's bare annotation
            // hides; an unbound `except` name falls back to the builtin.
            let local = table.unbound_locally(reference, name, builtins.contains(name));
            // Named by the source Ruff's range spans: a whole string for a name in one that is
            // not simple.
            let range = source_range(&table, reference);
            (table.unbound(reference, name) && (local || !builtins.contains(name)))
                .then(|| (range.start, source[range].to_owned()))
        })
        .collect::<BTreeSet<_>>();
    let theirs = expected["unresolved"]
        .as_array()
        .expect("unresolved")
        .iter()
        .map(|item| {
            (
                item["start"].as_u64().expect("start") as usize,
                item["name"].as_str().expect("name").to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    let line = |start: usize| source[..start].matches('\n').count() + 1;
    let mut differences = Vec::new();
    for (start, name) in ours.difference(&theirs) {
        differences.push(format!("only ours: `{name}` line {}", line(*start)));
    }
    for (start, name) in theirs.difference(&ours) {
        differences.push(format!("only the oracle's: `{name}` line {}", line(*start)));
    }
    // TypeScript: for each reference both resolve, whether the binding carries its initializer
    // (eslint-utils' `canBeConsideredConst`) and whether a property of its value is mutated. A
    // plain write counts as a mutation there; here it is a write. A computed call (`x[k]()`)
    // counts as a mutation here only.
    for item in expected["constants"].as_array().into_iter().flatten() {
        let start = item["start"].as_u64().expect("start") as usize;
        let name = item["name"].as_str().expect("name");
        let Some(reference) = table
            .references
            .binary_search_by_key(&start, |reference| reference.start)
            .ok()
            .map(|index| &table.references[index])
        else {
            continue;
        };
        let Some((binding, value)) = table.binding(reference, name) else {
            continue;
        };
        let constant = item["constant"].as_bool().expect("constant");
        // A destructured name's element is no initializer eslint-utils reads.
        let value = value.filter(|value| !binding.followed(*value).destructured);
        if value.is_some() != constant {
            differences.push(format!(
                "`{name}` line {}: a value {}, the oracle's {constant}",
                line(start),
                value.is_some()
            ));
        }
        let mutated = item["mutated"].as_bool().expect("mutated");
        if mutated && !binding.mutated && binding.writes == 0 {
            differences.push(format!(
                "`{name}` line {}: mutated only by the oracle",
                line(start)
            ));
        }
    }
    // TypeScript: each variable's references, eslint-scope's `variable.references` (reads, writes
    // and the writes at its definitions), by its first definition's name.
    // A class declaration's name is two variables, the outer one's and its class scope's: they
    // pair up in scope order.
    let mut theirs = std::collections::BTreeMap::<(usize, &str), Vec<u64>>::new();
    for item in expected["variables"].as_array().into_iter().flatten() {
        let key = (
            item["start"].as_u64().expect("start") as usize,
            item["name"].as_str().expect("name"),
        );
        theirs
            .entry(key)
            .or_default()
            .push(item["references"].as_u64().expect("references"));
    }
    if language != GrammarDialect::Python {
        let usage =
            usage::Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
        for ((start, name), references) in theirs {
            let ours = table
                .scopes
                .iter()
                .enumerate()
                .filter_map(|(scope, entry)| {
                    // A class scope's own binding of its declaration's name has no name node.
                    let index =
                        entry.bindings.get(name)?.iter().position(|binding| {
                            match binding.name {
                                Some(node) => node.start as usize == start,
                                None => {
                                    entry.kind == ScopeKind::Class
                                        && binding.site.node.is_some_and(|site| {
                                            site.start as usize <= start
                                                && start < site.end as usize
                                        })
                                }
                            }
                        })?;
                    let id = usage::BindingId {
                        scope: scope as u32,
                        name,
                        index,
                    };
                    Some(u64::from(usage.facts(id).references))
                })
                .collect::<Vec<_>>();
            if ours != references {
                differences.push(format!(
                    "`{name}` line {}: references {ours:?}, the oracle's {references:?}",
                    line(start)
                ));
            }
        }
    }
    differences
}

/// The oracle fixtures in `<pack>/scope/<directory>`; `SCOPE_ORACLE_ROOT` names another `scope` directory.
fn oracle_fixtures(
    pack: &str,
    directory: &str,
    extension: &str,
) -> Vec<(String, String, serde_json::Value)> {
    let root = std::env::var("SCOPE_ORACLE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("RERAIL_FIXTURES_DIR"))
                .join(pack)
                .join("scope")
        })
        .join(directory);
    let mut fixtures = std::fs::read_dir(&root)
        .expect("oracle directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|found| found == extension))
        .map(|path| {
            let source = std::fs::read_to_string(&path).expect("fixture source");
            let expected_path = path.with_extension("expected.json");
            let expected = std::fs::read_to_string(&expected_path).expect("expected output");
            (
                path.file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned(),
                source,
                serde_json::from_str(&expected).expect("expected JSON"),
            )
        })
        .collect::<Vec<_>>();
    fixtures.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(!fixtures.is_empty(), "no fixtures in {}", root.display());
    fixtures
}

#[test]
fn python_tables_agree_with_ruff_on_every_oracle_fixture() {
    let mut report = Vec::new();
    for (name, source, expected) in oracle_fixtures("python", "python", "py") {
        for difference in oracle_differences(GrammarDialect::Python, &source, &expected) {
            report.push(format!("{name}: {difference}"));
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}

#[test]
fn typescript_tables_agree_with_typescript_eslint_on_every_oracle_fixture() {
    let mut report = Vec::new();
    for extension in ["ts", "tsx"] {
        let language = if extension == "tsx" {
            GrammarDialect::Tsx
        } else {
            GrammarDialect::TypeScript
        };
        for (name, source, expected) in oracle_fixtures("ecma", "typescript", extension) {
            for dialect in [language, GrammarDialect::JavaScript] {
                let root = dialect.ast_grep(&source);
                if dialect == GrammarDialect::JavaScript
                    && root.root().get_doc().tree.root_node().has_error()
                {
                    continue;
                }
                for difference in oracle_differences(dialect, &source, &expected) {
                    report.push(format!("{dialect:?} {name}: {difference}"));
                }
            }
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}
