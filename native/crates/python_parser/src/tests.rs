//! The compiled grammar against the lean parser: the same tape, node for node, on every text it
//! reads; a refusal (to the lean parser) where it does not.

use super::*;

/// The tests' grammar: the crate's, or `RERAIL_PYTHON_GRAMMAR`'s file (a variant tried without a
/// rebuild). It shadows the crate's `compiled`.
fn compiled() -> Result<&'static Compiled, &'static str> {
    static OTHER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();
    match std::env::var("RERAIL_PYTHON_GRAMMAR") {
        Ok(p) => OTHER.get_or_init(|| compile_answered(&std::fs::read_to_string(p).expect("the grammar file"))).as_ref().map_err(String::as_str),
        Err(_) => super::compiled(),
    }
}

/// Each of `texts` read by the compiled grammar as the lean parser reads it: the JIT's tape, and
/// the interpreter's over the interpreter's tier 1.
fn same(texts: &[&str]) {
    let c = compiled().unwrap_or_else(|e| panic!("the grammar compiles: {e}"));
    for &text in texts {
        let want = lean_text(text).unwrap_or_else(|| panic!("the lean parser refuses {text:?}"));
        let image = c.parser.image(text.as_bytes()).unwrap_or_else(|why| panic!("the grammar refuses {text:?}: {why:?}\n{}", c.explain(text)));
        let got = image_text(&image).expect("a tape image");
        assert_eq!(got, want, "{text:?}");
        let run = c.program.block_programs[0].run(text.as_bytes());
        let nodes = rerail_compiler::stream::run(&c.program, text.as_bytes(), &run).unwrap_or_else(|why| panic!("the interpreter refuses {text:?}: {why:?}"));
        let interp = rerail_compiler::stream::image(&nodes, &c.program.readings.extras, TAG, text.as_bytes());
        assert_eq!(image_text(&interp).expect("a tape image"), want, "the interpreter, {text:?}");
        assert_eq!(rounds_differ(text.as_bytes()), None, "the regions as rounds, Metal's form, against the walk, {text:?}");
        if let Some(why) = tier1_differs(c, text) {
            panic!("tier 1 as Rust, {text:?}: {why}");
        }
        let aot = aot_text(text).unwrap_or_else(|at| panic!("the parser as Rust refuses {text:?} at byte {at}"));
        assert_eq!(aot, want, "the parser as Rust, {text:?}");
    }
}

/// The first output whose words differ between the regions walked and run as rounds (Metal's
/// form, `rounds_values`), with the first byte they differ at; `None` where they are one.
fn rounds_differ(bytes: &[u8]) -> Option<(usize, usize)> {
    let program = &compiled().expect("compiles").program.block_programs[0];
    let (walked, spans) = program.all_values(&[bytes]);
    let (rounds, _) = program.rounds_values(&[bytes]);
    program.outputs.iter().enumerate().find_map(|(k, &v)| {
        let (a, b) = (&walked[usize::from(v)][spans[0].clone()], &rounds[usize::from(v)][spans[0].clone()]);
        a.iter().zip(b).position(|(x, y)| x != y).map(|i| (k, 64 * i + (a[i] ^ b[i]).trailing_zeros() as usize))
    })
}

/// Each of `texts` refused by the compiled grammar (to the lean parser).
fn refused(texts: &[&str]) {
    let c = compiled().expect("compiles");
    for &text in texts {
        assert!(c.image(text).is_none(), "accepted {text:?}");
        assert!(aot_text(text).is_err(), "the parser as Rust accepts {text:?}");
        if let Some(why) = tier1_differs(c, text) {
            panic!("tier 1 as Rust, {text:?}: {why}");
        }
    }
}

#[test]
fn simple_statements_are_the_lean_parsers() {
    same(&["x\n", "x\ny\n", "pass\n", "1\n", "x; y\n", "x;\n", "x", "", "\n\nx\n\n", "# c\nx  # d\n# e\n", "x\n# e"]);
    // The module: from its first token or comment, to past the blanks after its last, but no line break.
    same(&["x\n\n\n", "x  \n", "x\n  \n", "\nx", "  \nx\n", "x  # c\n\n"]);
}

#[test]
fn blocks_are_the_lean_parsers() {
    same(&["if a:\n    b\n", "if a:\n    b\n    c\nd\n", "if a:\n  b\nelif c:\n  d\nelse:\n  e\n", "while a:\n  b\nelse:\n  c\n"]);
    // Nested, and a dedent past two blocks at once, or to between them.
    same(&["if a:\n  if b:\n    c\nd\n", "if a:\n  if b:\n    c\n  d\ne\n", "if a:\n    b\n  c\n", "if a:\n  b\n      c\n"]);
    // The file ends in a block.
    same(&["if a:\n  b", "if a:\n  if b:\n    c"]);
    // A comment before a statement's line break: the block's, where the line ends it or the file does.
    same(&["if a:\n  b  # c\n", "if a:\n  b  # c\nd\n", "if a:\n  b  # c\n  d\n", "if a:\n  b  # c", "if a:\n  if b:\n    c  # d\n", "if a:\n  if b:\n    c  # d\n  e\n"]);
    // A comment line before a dedent: each block's as deep as it, up to the first one less deep
    // (tree-sitter's scanner dedents past them); one less deep than the next line is the block's.
    same(&["if a:\n  b\n  # c\nd\n", "if a:\n  b\n# c\nd\n", "if a:\n  b\n  # c\n", "if a:\n  if b:\n    c\n  # d\ne\n", "if a:\n  b\n  # c\n# d\ne\n"]);
    same(&["if a:\n  b\n# c\n  d\n", "if a:\n  b\n      # c\nd\n"]);
    // A dedent past two levels ends both: the `else` after it is the outer statement's, not the inner `while`'s or `if`'s.
    same(&["if a:\n  while b:\n    c\nelse:\n  d\n", "if a:\n  if b:\n    c\n  elif d:\n    e\nelse:\n  f\n", "try:\n  for a in b:\n    c\nexcept E:\n  d\n"]);
}

#[test]
fn strings_are_the_lean_parsers() {
    same(&["\"a\"\n", "'a\\nb'\n", "r'a\\d'\n", "b\"x\"\n", "\"\"\"a\nb\"\"\"\n", "'''a'''\n", "''\n", "r''\n", "''''''\n"]);
    // Escapes: each of lean's, a lone backslash (a leaf), a quote and a line break escaped.
    same(&["'\\''\n", "'a\\\nb'\n", "\"\\x41\\u1234\\U0001F600\"\n", "'\\d'\n", "'\\1\\12\\123\\1234'\n", "'\\\\'\n", "b'\\x00'\n", "'\\u12'\n"]);
    // A comment's `#` in a string, quotes of the other kind, a comment after one; bytes past 0x7f;
    // prefixes; a raw string's escaped quote, and its backslashes before the closer.
    // An `f'` or a word of prefix letters before a quote, in a string or a comment: no prefix.
    same(&["\"a f'\"\n", "x  # but's\n", "'(f'\n"]);
    same(&["\"#\"\n", "'''a'b''c'''\n", "'a'  # c\n", "\"\u{e9}\"\n", "Rb'x'\n", "u'x'\n", "r'a\\''\n", "r'a\\\\'\n"]);
}

#[test]
fn statements_are_the_lean_parsers() {
    same(&["return\n", "return x\n", "raise X from e\n", "raise\n", "del a, b\n", "del a,\n", "assert x, y\n", "global a, b\n", "nonlocal a\n", "break\n", "continue\n", "return a, b\n"]);
    // Assignments: the target moved into `left`, a pattern.
    same(&["x = y = 1\n", "a = b = c\n", "a: int = 1\n", "a: int\n", "a += 1\n", "a.b[0] = 1\n", "x = 'a'\n", "x = a, b\n"]);
    // An annotation's type: a generic type (a name before `[`), else an expression.
    same(&["x: list[str]\n", "x: a.b\n", "x: Optional[a.B] = None\n", "x: dict[str, int]\n", "x: \"str\"\n", "x: list[str,]\n", "x: f()\n", "x = a[b]\n"]);
}

#[test]
fn imports_are_the_lean_parsers() {
    same(&["import a\n", "import a.b as c, d\n", "from a.b import c as d, e\n", "from . import x\n", "from ..a import (b, c,)\n", "from a import *\n"]);
    same(&["from __future__ import annotations\n", "from a import (\n    b,\n    c,\n)\n", "from a import (b)\n", "from a import b, c\n", "import __future__\n"]);
}

#[test]
fn primaries_are_the_lean_parsers() {
    // A primary's postfix operations: each wraps the primary so far, which moves into its slot.
    same(&["a.b.c\n", "f(x, y=1, *a, **k)\n", "a[1]\n", "a.b(c)[d]\n", "f(x=1)\n", "print(a)\n", "f()\n"]);
    // A trailing comma and its closer are leaves.
    same(&["f(a,)\n", "f(a, b,)\n", "x = [a, b,]\n", "x = (a, b,)\n", "x = (a,)\n", "x = ()\n", "x = (a)\n", "x = []\n", "x = True, False, None\n"]);
}

#[test]
fn definitions_are_the_lean_parsers() {
    same(&["class A:\n    pass\n", "class A(B, metaclass=M):\n    x = 1\n", "class A():\n    pass\n", "class A(B,):\n    pass\n", "def f():\n    pass\n", "async def f():\n    pass\n"]);
    // Parameters: a name, a default, a type, both, splats, separators; a typed splat.
    same(&["def f(a, b=1, *args, c: int = 2, **kw) -> int:\n    return a\n", "def f(a, /, b, *, c):\n    pass\n", "def f(a: int, b,):\n    pass\n"]);
    same(&["def f(*a: int, **k: str):\n    pass\n", "def f(*, a):\n    pass\n", "def f(a=1, b: int = 2):\n    pass\n"]);
    same(&["@d\ndef f():\n    pass\n", "@a.b(c)\nclass A:\n    pass\n", "@a\n@b\ndef f():\n    pass\n"]);
}

#[test]
fn dictionaries_are_the_lean_parsers() {
    same(&["x = {}\n", "x = {\"a\": 1, \"b\": 2,}\n", "x = {**a}\n", "x = {a, b}\n", "x = {a,}\n", "x = {\"a\": 1}\n", "x = {*a, b}\n"]);
}

#[test]
fn comprehensions_are_the_lean_parsers() {
    same(&["x = [a for a in b]\n", "x = [a for a in b if c if d for e in f]\n", "x = {a: b for a, b in c}\n", "x = {a for a in b}\n"]);
    same(&["f(a for a in b)\n", "x = (a for a in b)\n", "x = [a.b for a in c if not a.d]\n", "x = [a for a in b if c and d or e]\n"]);
    same(&["x = [a if b else c for a in d]\n", "x = [*a, *b]\n", "x = [*a, b]\n", "x = [a, *b]\n", "x = []\n", "f()\n", "x = [a async for a in b]\n"]);
    // A list whose one item is a splat: tree-sitter's splat is a primary there (`star_item`).
    same(&["x = [a for s in [*b]]\n", "x = [a for s in [*b.c()]]\n", "for s in [*b]:\n    a\n"]);
}

#[test]
fn await_ellipsis_and_async_are_the_lean_parsers() {
    same(&["x = await a.b(c)\n", "await a\n", "x = [await a for a in b]\n", "x: tuple[int, ...] = ...\n", "def f():\n    ...\n", "x = a[...]\n"]);
    same(&["async def f():\n    async for a in b:\n        c\n    async with a as b:\n        c\n", "async def f() -> int:\n    a\n"]);
    same(&["from . import a\n", "from .. import a\n"]);
}

#[test]
fn targets_slices_lambdas_yields_and_concatenations_are_the_lean_parsers() {
    same(&["a, b = c\n", "a, b = c, d\n", "a, = c\n", "a, *b = c\n", "a.x, b[0] = c\n", "a, b, = c\n"]);
    same(&["x = a[1:2]\n", "x = a[:2]\n", "x = a[1:]\n", "x = a[::2]\n", "x = a[:, 0]\n", "x = a[1:-1]\n", "x = a[:]\n", "a[1:] = b\n", "x = a[b, c,]\n", "x = a[b,]\n"]);
    same(&["x = lambda: a\n", "x = lambda a, *b, **c: a\n", "f(key=lambda k: k)\n", "x = lambda a=1: a if a else b\n"]);
    same(&["def f():\n    yield a\n", "def f():\n    yield\n", "def f():\n    x = yield a\n", "def f():\n    yield from a\n", "def f():\n    yield a, b\n"]);
    same(&["x = \"a\" \"b\"\n", "x = (\"a\"\n     \"b\")\n", "x = \"a\" r\"b\" f\"{c}\"\n"]);
    // In a block: a region's first part takes its record's marks, not its line break's.
    same(&["def f():\n    x = \"a\" \"b\"\n", "def f():\n    x = (\n        \"a\"\n        \"b\"\n    )\n", "if a:\n    f(\n        \"a\"\n        \"b\"\n    )\n"]);
    // Not read yet: a bare list of expressions as a statement, a tuple as a target.
    refused(&["a, b\n"]);
}

#[test]
fn octal_and_binary_numbers_and_quotes_in_strings_are_the_lean_parsers() {
    same(&["x = 0o600\n", "x = 0o17\n", "x = 0b101 + 0O7 + 0B1\n", "x = 0o1_7\n", "x = \"a\'\'\'\'b\"\n", "x = \'a\"\"\"\"b\'\n", "x = \"\"\"a\'\'\'\'b\"\"\"\n"]);
    refused(&["x = 0o8\n", "x = 0b2\n", "x = 0o\n"]);
}

#[test]
fn f_strings_in_an_interpolation_are_the_lean_parsers() {
    same(&["x = f\"{', '.join(f'{a}' for a in b)}\"\n", "x = f\"a{f'b{c}d'}e{g}\"\n", "x = f\"{f'{f\"{a}\"}'}\"\n", "x = f\"{f\"{a}\"}\"\n"]);
    same(&["x = f\"\"\"{f'{a}'}\n\"\"\"\n", "x = f\"{a}\" + f'{f\"{b}\"}{c}'\n", "def f():\n    return f\"{g(f'{x}', y)}\"\n    z\n"]);
    refused(&["x = f\"{f'{f\"{f'{f\"{a}\"}'}\"}'}\"\n"]);
}

#[test]
fn brackets_in_an_interpolation_are_the_lean_parsers() {
    same(&["x = f\"{a[1:2]}\"\n", "x = f\"{a[b[1]:2]:>4}\"\n", "x = f\"{(lambda y: y)(1)}\"\n", "x = f\"{a[:]} {b}\"\n"]);
    same(&["x = f\"{ {'k': v} }\"\n", "x = f\"{g({'a': 1})}?q\"\n", "x = f\"{d['k']:x}\"\n", "x = f\"\"\"{\n  {1: 2}\n}\"\"\"\n", "x = f\"{ {a} }\"\n"]);
    // `max` bounds the frames of every template open, the templates' own among them: two
    // templates and 5 brackets are 7; one bracket more is past it.
    let nested = |n: usize| format!("x = f\"{{f'{{{}a{}}}'}}\"\n", "(".repeat(n), ")".repeat(n));
    same(&[nested(5).as_str()]);
    refused(&[nested(6).as_str()]);
    // A close with a template's frame on top: no bracket of the code's to close.
    refused(&["x = f\"{a)}\"\n", "x = f\"{f'{a]}'}\"\n"]);
}

#[test]
fn braces_raw_f_strings_and_line_breaks_in_interpolations_are_the_lean_parsers() {
    same(&["x = f\"{{{a}}}\"\n", "x = f\"a {{{b}}} c\"\n", "x = f\"{{{{a}}}}\"\n", "x = f\"{{a}}\"\n"]);
    same(&["x = rf\"a\\\\d{b}\\\\w\"\n", "x = fr\'{a}\\\\n\'\n", "x = rf\"{a}\\\\n{b}\\\\t\"\n", "x = rf\"{a}\" + f\"{b}\\\\n\"\n"]);
    same(&["x = f\"\"\"{\n    a\n}\"\"\"\n", "x = f\"\"\"a{\n  b(\n    c,\n  )\n}d\"\"\"\n", "def f():\n    x = f\"\"\"{\n        a\n    }\"\"\"\n    y\n"]);
    same(&["match a:\n    case X(b, c,):\n        d\n", "match a:\n    case X(\n        b=1,\n    ):\n        d\n"]);
    refused(&["x = f\"a}}}\"\n"]);
}

#[test]
fn match_statements_are_the_lean_parsers() {
    same(&["match a:\n    case 1:\n        b\n    case _:\n        c\n", "match a.b:\n    case X.Y:\n        b\n    case None:\n        c\n"]);
    same(&["match a:\n    case list() | tuple():\n        b\n    case str(x):\n        c\n", "match a:\n    case [b, *c]:\n        d\n    case {\"k\": v, **r}:\n        e\n"]);
    same(&["match a:\n    case (b, c) if b > c:\n        d\n    case B(x=1) as y:\n        e\n", "match a:\n    case b:\n        c\n", "match a:\n    case 1 | 2 as b:\n        c\n"]);
    same(&["match (a, b):\n    case (1, 2):\n        c\n", "match a:\n    case \"x\" | \"y\":\n        b\n    case [1, 2, *_]:\n        c\n", "match a:\n    case -1:\n        b\n    case {\"k\": [x, y]}:\n        c\n"]);
    same(&["match a:\n    case X(b=c, d=e):\n        f\n    case X(1, 2):\n        g\n", "def f():\n    match a:\n        case 1:\n            b\n    return c\n"]);
    // The body from the end of the record before its first `case`: the `:`'s, or a comment's.
    same(&["match a:   \n    case 1: b\n", "match a:  # c\n    case 1:\n        b\n"]);
    same(&["match a:\n    case X(value=int() | float() as v):\n        b\n", "match a:\n    case X(value=1 | 2):\n        b\n", "match a:\n    case X(value=[b]):\n        c\n"]);
    same(&["match a:\n    case [b as c]:\n        d\n", "match a:\n    case 1, 2:\n        b\n", "match a, b:\n    case c:\n        d\n"]);
    // `match` as a name, an annotation's too.
    same(&["match: int = 1\n", "match = 1\n", "match(a)\n", "match.group(1)\nif a:\n    b\n", "x = match\n", "for _ in a:\n    b\n", "_ = 1\n"]);
}

#[test]
fn star_items_type_aliases_and_small_forms_are_the_lean_parsers() {
    same(&["x = [*a]\n", "x = [*a.b()]\n", "x = [*a, *b]\n", "x = [*f(a, b), c]\n", "x = *a, b\n", "x = a, *b\n", "return *a, b\n", "for x in *a, *b:\n    c\n"]);
    same(&["*a, b = c\n", "((), b) = c\n", "try:\n    a\nexcept* E as e:\n    b\n", "f(lambda a, *, b=None: c)\n"]);
    same(&["type X = int\n", "type X[T] = list[T]\n", "type = 1\n", "type(a)\n", "x = type\n"]);
}

#[test]
fn walrus_bodies_splats_patterns_and_type_parameters_are_the_lean_parsers() {
    same(&["if (a := b):\n    c\n", "if a := b:\n    c\n", "x = [y for z in w if (y := f(z))]\n", "while (a := f()) is not None:\n    b\n"]);
    same(&["def f(): ...\n", "def f() -> int: ...\n", "class A: pass\n", "if a: b\n", "while a: b; c\n", "if a: b\nelse: c\n", "class A:\n    def f(self): ...\n    x = 1\n"]);
    same(&["x = (*a, b)\n", "x = (a, *b)\n", "x = (*a,)\n", "f(*a)\n", "for a, *b in c:\n    d\n"]);
    same(&["for (a, b) in c:\n    d\n", "for i, (a, b) in c:\n    d\n", "for [a, b] in c:\n    d\n", "[a] = b\n", "[[a]] = b\n", "(a,) = b\n", "(a, b) = c\n", "[a, b] = c, d\n"]);
    same(&["x = (a) == b\n", "(a)\n", "[a, b]\n", "(a).b = c\n"]);
    same(&["class A[T]:\n    pass\n", "def f[T](a: T) -> T:\n    pass\n"]);
    // Not read yet: a statement's splat target, a splat in an expression list, a tuple pattern before a comma.
    refused(&["(a, b), c = d\n"]);
}

#[test]
fn types_are_the_lean_parsers() {
    same(&["x: int | None = None\n", "x: int | None\n", "def f(a: int | None = None) -> str | None:\n    a\n", "x: Optional[int] = None\n"]);
    // A union over a generic is a `union_type`, left to right; any other `|` is an expression's.
    same(&["x: list[int] | None = None\n", "def f(a: dict[str, int] | None = None):\n    a\n", "x: list[int] | dict[str, int] | None\n"]);
    same(&["x: int | list[int]\n", "x: list[int | None]\n", "def f() -> list[int] | None:\n    a\n"]);
}

#[test]
fn try_with_and_for_are_the_lean_parsers() {
    same(&["try:\n    a\nexcept E as e:\n    b\nexcept:\n    c\nelse:\n    d\nfinally:\n    e\n", "try:\n    a\nexcept (A, B):\n    b\n", "try:\n    a\nfinally:\n    b\n"]);
    same(&["with a as b, c:\n    d\n", "with a(x) as b:\n    d\n", "with a as b.c:\n    d\n"]);
    // A parenthesized clause: a comma inside with more after it, and the `:` after.
    same(&["with (\n    a as b,\n    c as d,\n):\n    e\n", "with (a, b):\n    c\n", "with (a(), b()):\n    c\n", "with (a as b, c as d):\n    e\n", "with (a,):\n    c\n", "with (a):\n    c\n"]);
    same(&["for i in range(3):\n    a\nelse:\n    b\n", "for a, b in c:\n    d\n", "for a.b in c:\n    d\n", "for a in b, c:\n    d\n", "for a[0] in b:\n    c\n"]);
    // Not read yet: an `as` in an expression's parentheses, a tuple or list as a loop's target.
    refused(&["with (a as b):\n    c\n", "with (a, b) as c:\n    d\n"]);
}

#[test]
fn assignment_targets_are_the_lean_parsers() {
    same(&["a.b = c\n", "a[0] = c\n", "a = b = c\n", "a.b += 1\n"]);
    // A call as a target: no pattern at all. A parenthesized name: a `tuple_pattern`, as the lean parser's.
    refused(&["f() = 1\n"]);
    same(&["(a) = b\n"]);
}

#[test]
fn f_strings_are_the_lean_parsers() {
    same(&["x = f\"a{b}c\"\n", "x = f\"{b}\"\n", "x = f\"a{b!r}\"\n", "x = f\"{b:>10}\"\n", "x = f\"{a.b(c)}\"\n", "x = f'{a[\"k\"]}'\n"]);
    same(&["x = f\"{{x}}\"\n", "x = f\"\"\"a\n{b}\n\"\"\"\n", "x = f\"{a + 1}\"\n", "x = F'{a}{b}' + f\"{c:%Y-%m-%d} {d!s:>3}\"\n", "x = f\"\"\n", "x = f\"{a[1]}:{b}\"\n"]);
    same(&["def f():\n    return f\"{a}\"\n", "x = {'a': f'{b}'}\n"]);
    // Not read yet: a nested spec, `=`, an empty interpolation or spec, a `}` alone.
    refused(&["x = f\"{x:}\"\n", "x = f\"{b:{w}}\"\n", "x = f\"{b=}\"\n", "x = f\"{}\"\n", "x = f\"a}\"\n"]);
}

#[test]
fn numbers_are_the_lean_parsers() {
    same(&["x = 1j\n", "x = 1.5j\n", "x = 0x1F\n", "x = 1_000\n", "x = 1.\n", "x = .5\n", "x = 1e5\n", "x = 1.5e-3\n", "x = 1E+5\n", "x = 0XAB\n"]);
    same(&["x = 1_0.2_5\n", "x = a.b + 1.5\n", "x = 2e-3 - e - 5\n", "x = [1.5, .25]\n", "x = f(1.)\n", "x = 0\n", "x = 007\n", "x = 0x_1\n"]);
}

#[test]
fn operators_are_the_lean_parsers() {
    same(&["x = a if b else c\n", "x = -a ** b\n", "x = a ** -b\n", "x = ~a\n", "x = a not in b\n", "x = a is not b\n", "x = a < b <= c\n", "x = -1\n"]);
    // `^` binds tighter than `&`; each operator left to right but `**`.
    same(&["x = a | b & c ^ d\n", "x = a << b + c\n", "x = a // b @ c\n", "x = (a + b) * c\n", "x = a ** b ** c\n", "x = a - b - c\n", "x = -a * b\n"]);
    same(&["x = not a == b\n", "x = a or not b\n", "x = a and b and c\n", "x = a or b and c\n", "x = a and b or c\n", "x = a if b else c if d else e\n", "x: a | None = None\n"]);
    same(&["if a is not None:\n    b\n", "x = a in b == c\n", "x = a is b\n", "x = not not a\n", "f(a + 1, b=c or d)\n", "x = a[b + 1]\n"]);
}

#[test]
fn what_the_grammar_does_not_read_is_refused() {
    refused(&["x.5\n", "x\\\n", "\tx\n", "x = 1L\n", "x = 1e5e3\n", "x = 1__0\n", "x = 0x\n"]);
    // A template string, a `\N{…}`, a raw string of backslashes only (its end starts at them), an open
    // string, a triple closed by four quotes, a three-letter prefix, `\u` in bytes.
    refused(&["t'{x}'\n", "u\"\\N{DASH}\"\n", "r'\\\\'\n", "r'\\''\n", "'a\n", "'''a''''\n", "rbu'x'\n", "b'\\u1234'\n"]);
}

/// The corpus: a folder of `.py` files, each read by both. `same`: the compiled grammar's tape is
/// the lean parser's; `refused`: it leaves the file to the lean parser; `differ`: a wrong tape,
/// which must stay 0.
#[test]
#[ignore = "reads RERAIL_PYTHON_CORPUS, a folder of .py files"]
fn corpus() {
    let dir = std::env::var("RERAIL_PYTHON_CORPUS").expect("set RERAIL_PYTHON_CORPUS");
    let c = compiled().expect("compiles");
    let (mut same, mut refused, mut differ, mut lean_refused, mut rounds_off) = (0, 0, 0, 0, 0);
    // The parser as Rust where the interpreter is the lean parser: its tier 1 the interpreter's, then its tape.
    let (mut aot_same, mut aot_off) = (0usize, 0usize);
    let mut taken: std::collections::BTreeMap<(usize, u32), usize> = Default::default();
    let (mut voted, mut alone) = (0usize, 0usize);
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();
    let mut paths = Vec::new();
    let mut dirs = vec![std::path::PathBuf::from(&dir)];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "py") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Some(want) = lean_text(&text) else {
            lean_refused += 1;
            continue;
        };
        match c.image(&text) {
            None => {
                refused += 1;
                if std::env::var_os("RERAIL_PYTHON_WHY").is_some() {
                    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
                    let tables = Default::default();
                    let w = match ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &tables) {
                        Ok(out) => match out.errors.first() {
                            Some(e) => {
                                if std::env::var_os("RERAIL_PYTHON_PATHS").is_some() {
                                    eprintln!("refused-at {e} :: {}", p.display());
                                }
                                e.split(" (byte").next().unwrap_or(e).to_string()
                            }
                            None => match c.refused_at(&text) {
                                Some(a) => {
                                    let a = a as usize;
                                    // `RERAIL_PYTHON_NEAR=N`: N characters from the refusal, and 30 before it.
                                    let n = std::env::var("RERAIL_PYTHON_NEAR").ok().and_then(|v| v.parse().ok()).unwrap_or(12);
                                    let before: String = if n > 12 { text[..a].chars().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect::<String>() + "‸" } else { String::new() };
                                    let near: String = (before + &text[a..].chars().take(n).collect::<String>()).replace('\n', "⏎");
                                    format!("the grammar at `{near}`")
                                }
                                None => "the grammar".into(),
                            },
                        },
                        Err(e) => format!("eval: {e:?}"),
                    };
                    // `RERAIL_PYTHON_PATHS`: each refused file's path beside its reason.
                    if std::env::var_os("RERAIL_PYTHON_PATHS").is_some() {
                        eprintln!("refused-path {w} :: {}", p.display());
                    }
                    *why.entry(w).or_default() += 1;
                }
            }
            Some(image) if image_text(&image).as_deref() == Ok(&want) => {
                same += 1;
                let aot = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tier1_differs(&c, &text).map_or_else(|| aot_text(&text).map_err(|at| format!("refused at byte {at}")), Err)));
                match aot {
                    Ok(Ok(t)) if t == want => aot_same += 1,
                    Ok(Ok(got)) => {
                        aot_off += 1;
                        let at = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(0);
                        let line = |t: &str| t.lines().nth(at).unwrap_or("").to_owned();
                        let before = want.lines().nth(at.saturating_sub(1)).unwrap_or("").trim().to_owned();
                        eprintln!("aot-differs tape: `{}` for `{}` after `{before}` :: {}", line(&got).trim(), line(&want).trim(), p.display());
                    }
                    Ok(Err(e)) => {
                        aot_off += 1;
                        eprintln!("aot-differs {} :: {}", e.chars().take(200).collect::<String>(), p.display());
                    }
                    Err(e) => {
                        aot_off += 1;
                        let m = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                        eprintln!("aot-panics {m} :: {}", p.display());
                    }
                }
                // The regions as rounds (Metal's form) against the walk, and the rounds each block took.
                if let Some((k, at)) = rounds_differ(text.as_bytes()) {
                    rounds_off += 1;
                    eprintln!("rounds-differ output {k} at byte {at} :: {}", p.display());
                }
                let rounds = c.program.block_programs[0].rounds_taken(&[text.as_bytes()]);
                for &(g, r) in &rounds {
                    *taken.entry((g, r)).or_default() += 1;
                }
                // Metal's SIMD group runs 32 of a stream's blocks together, to the rounds of the slowest (a vote).
                for w in rounds.chunks(32) {
                    voted += w.iter().map(|x| x.1).max().unwrap_or(0) as usize * w.len();
                    alone += w.iter().map(|x| x.1 as usize).sum::<usize>();
                }
            }
            Some(image) => {
                differ += 1;
                let got = image_text(&image).unwrap_or_default();
                let (have, wanted): (Vec<&str>, Vec<&str>) = (got.lines().collect(), want.lines().collect());
                let at = have.iter().zip(&wanted).position(|(a, b)| a != b).unwrap_or(have.len().min(wanted.len()));
                let from = at.saturating_sub(4);
                eprintln!("differs: {}\n  context:\n    {}\n  compiled:\n    {}\n  lean:\n    {}", p.display(), wanted[from..at].join("\n    "), have[at..(at + 3).min(have.len())].join("\n    "), wanted[at..(at + 3).min(wanted.len())].join("\n    "));
            }
        }
    }
    let mut why: Vec<(usize, String)> = why.into_iter().map(|(w, n)| (n, w)).collect();
    why.sort_unstable_by(|a, b| b.cmp(a));
    // `RERAIL_PYTHON_WHY=N`: the N most frequent reasons (40 where it is no number).
    let shown = std::env::var("RERAIL_PYTHON_WHY").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    for (n, w) in why.iter().take(shown) {
        eprintln!("refused {n:5}: {w}");
    }
    // Each rounds group's operations, and how many blocks took each number of rounds.
    let program = &c.program.block_programs[0];
    let groups = rerail_compiler::block::groups(&program.ops, &program.rounds).expect("verified");
    for (g, group) in groups.iter().enumerate() {
        let blocks: Vec<String> = taken.iter().filter(|((h, _), _)| *h == g).map(|((_, r), n)| format!("{r}: {n}")).collect();
        eprintln!("rounds group {g}: {} operations, {} carries; blocks by rounds {}", group.members.len(), group.carries.len(), blocks.join(", "));
    }
    let blocks: usize = taken.values().sum();
    eprintln!("rounds a block: {:.2} alone, {:.2} voted in 32s; {blocks} blocks", alone as f64 / blocks.max(1) as f64, voted as f64 / blocks.max(1) as f64);
    eprintln!("python corpus: {same} the same, {refused} refused, {differ} differ ({lean_refused} the lean parser refuses); {rounds_off} whose rounds differ from the walk");
    eprintln!("python corpus as Rust: {aot_same} of the {same} the same, {aot_off} not");
    assert_eq!(differ, 0);
    assert_eq!(aot_off, 0);
    assert_eq!(rounds_off, 0);
}

/// `rerail-python-aot`'s tiers are the compiler's output (`RERAIL_BLESS=1` writes them again).
#[test]
fn rust_is_current() {
    let grammar = std::env::var("RERAIL_PYTHON_GRAMMAR").map_or_else(|_| GRAMMAR.to_owned(), |p| std::fs::read_to_string(p).expect("the grammar file"));
    let (tier1, tier2) = crate::tiers_rust(&grammar).unwrap_or_else(|e| panic!("the parser as Rust: {e}"));
    for (file, rust) in [("tier1.rs", tier1), ("tier2.rs", tier2)] {
        let path = format!("{}/../python_aot/src/{file}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("RERAIL_BLESS").is_some() {
            std::fs::write(&path, &rust).expect("writes");
        }
        assert!(std::fs::read_to_string(&path).is_ok_and(|have| have == rust), "python_aot's {file} is not the compiler's: RERAIL_BLESS=1");
    }
}

/// Where tier 1 as Rust (`rerail_python_aot::lex`) is not the interpreter's tier 1: whether it
/// sends the file to tree-sitter, and where neither does, each token's first and last byte and
/// code.
fn tier1_differs(c: &Compiled, text: &str) -> Option<String> {
    let run = c.program.block_programs[0].run(text.as_bytes());
    let lx = rerail_python_aot::lex(text.as_bytes());
    let refused = run.totals.first().is_some_and(|&t| t != 0);
    // An escape of `A & ~L` the lowering makes a local check of tier 2's at `A`'s token
    // (`pending_checks`): the interpreter refuses the file there, tier 1 as Rust where it reads
    // the escape. Either way the file is refused.
    if lx.refused && !refused && c.parser.image(text.as_bytes()).is_err() {
        return None;
    }
    if refused != lx.refused {
        return Some(format!("refused: the interpreter {refused}, tier 1 as Rust {}", lx.refused));
    }
    if refused {
        return None;
    }
    let (starts, lasts) = (&run.positions[0], &run.positions[1]);
    // Tier 1 as Rust keeps the comments apart from the tokens (as the lean lexer); the interpreter's records hold both,
    // by where they start.
    let mut recs: Vec<_> = lx.recs.iter().chain(&lx.extras).copied().collect();
    recs.sort_by_key(|r| r.start);
    let n = starts.len().max(recs.len());
    for k in 0..n {
        let theirs = (starts.get(k), lasts.get(k), run.codes.get(k));
        let rec = recs.get(k);
        let ours = (rec.map(|r| &r.start), rec.map(|r| &r.last), rec.map(|r| &r.code));
        if theirs != ours {
            let at = |s: Option<&u32>| s.map_or(String::new(), |&s| text[s as usize..].chars().take(16).collect::<String>().replace('\n', "⏎"));
            return Some(format!("record {k}: the interpreter {theirs:x?} `{}`, tier 1 as Rust {ours:x?} `{}`", at(theirs.0), at(ours.0)));
        }
    }
    None
}

/// `text`'s tape from the parser as Rust (`rerail_python_aot`: tier 1, then tier 2), as text: its keys the tape
/// format's, read as `image_text` reads an image's code table.
fn aot_text(text: &str) -> Result<String, u32> {
    use rerail_tape_format::{CLASS_SHIFT, CODE_EXTRA, KIND_BITS, TAIL_FROM_START};
    let t = rerail_python_aot::parse(text.as_bytes())?;
    let cell = |column: &[u8], width: u8, i: usize| -> u32 {
        let w = usize::from(width & 0x7f);
        let b = &column[i * w..i * w + w];
        if w == 2 { u32::from(u16::from_le_bytes([b[0], b[1]])) } else { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
    };
    let start = |i: usize| {
        let b = i / rerail_tape_format::CHECKPOINT_STRIDE;
        u32::from_le_bytes(t.base[4 * b..4 * b + 4].try_into().expect("a base")) + cell(&t.off, t.widths[1], i)
    };
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..t.code.len() / 2 {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let key = t.keys[usize::from(u16::from_le_bytes([t.code[2 * i], t.code[2 * i + 1]]))];
        let (kind, field) = (key & ((1 << KIND_BITS) - 1), key >> KIND_BITS & 0x3f);
        let mut classes: Vec<&str> = (0..rerail_python_aot::SUPERS.len())
            .filter(|j| key >> (CLASS_SHIFT + *j as u32) & 1 != 0)
            .map(|j| rerail_python_aot::SUPERS[j].checked_sub(SUPERTYPE_IDS).and_then(|c| SUPERTYPES.get(usize::from(c))).copied().unwrap_or("?"))
            .collect();
        classes.sort_unstable();
        let size = cell(&t.size, t.widths[0], i) as usize;
        let s = start(i);
        let from = if t.widths[2] & TAIL_FROM_START != 0 { s } else { start(i + size - 1) };
        let end = from + cell(&t.tail, t.widths[2], i);
        out.push_str(&format!(
            "{}{}{}{}{} [{s}..{end}]\n",
            "  ".repeat(open.len()),
            lean::KINDS.get(kind as usize).map_or("?", |k| k.0),
            if field == 0 { String::new() } else { format!(" {}:", lean::FIELDS.get(field as usize).copied().unwrap_or("?")) },
            if (key >> 16) as u8 & CODE_EXTRA != 0 { " flags=1" } else { "" },
            if classes.is_empty() { String::new() } else { format!(" {{{}}}", classes.join(",")) },
        ));
        open.push(i + size);
    }
    Ok(out)
}
