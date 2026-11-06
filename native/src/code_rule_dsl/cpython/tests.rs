use super::syntax::syntax_error;

/// `ast.parse`'s verdict, line and message, as CPython 3.13.18 gives them (`python3.13 -I -S`, on nera-port-4).
#[test]
fn syntax_errors_are_ast_parses() {
    let cases: &[(&str, Option<(usize, &str)>)] = &[
        ("if True:\n", Some((1, "expected an indented block after 'if' statement on line 1"))),
        ("def f():\n", Some((1, "expected an indented block after function definition on line 1"))),
        ("return 1\n", None),
        ("x = (1,\n", Some((1, "'(' was never closed"))),
        ("x = '\n", Some((1, "unterminated string literal (detected at line 1)"))),
        ("\0", Some((1, "source code string cannot contain null bytes"))),
        ("\u{feff}x = 1\n", Some((1, "invalid non-printable character U+FEFF"))),
        ("a b\n", Some((1, "invalid syntax"))),
        ("def f(a, a): pass\n", None),
        ("f(a=1, a=2)\n", None),
        ("try:\n  pass\nexcept A, B:\n  pass\n", Some((3, "multiple exception types must be parenthesized"))),
        ("x = t'a'\n", Some((1, "invalid syntax"))),
        ("*a = 1\n", None),
        ("del f()\n", Some((1, "cannot delete function call"))),
        ("  x = 1\n", Some((1, "unexpected indent"))),
        ("if 1:\n  x\n y\n", Some((3, "unindent does not match any outer indentation level"))),
        ("x = [1, 2\ny = 3\n", Some((1, "'[' was never closed"))),
        ("lambda: (yield)\n", None),
        ("nonlocal x\n", None),
        ("break\n", None),
        ("from __future__ import braces\n", None),
        ("await x\n", None),
        ("x = 1 +\n", Some((1, "invalid syntax"))),
        ("class C\n", Some((1, "expected ':'"))),
        ("f(x for x in y, 1)\n", Some((1, "Generator expression must be parenthesized"))),
        ("a, *b, *c = d\n", None),
        ("x: int = 1 = 2\n", Some((1, "invalid syntax"))),
        ("def f(:\n    pass\n", Some((1, "invalid syntax"))),
        ("def f(o):\n    # c\n", Some((2, "expected an indented block after function definition on line 1"))),
        ("x = 1\nif x:\n# nothing\n", Some((3, "expected an indented block after 'if' statement on line 2"))),
        ("x = [1 2]\n", Some((1, "invalid syntax. Perhaps you forgot a comma?"))),
        ("from x import (a, b\n1 + 1\n", Some((1, "'(' was never closed"))),
        ("s = '''abc\n\n", Some((1, "unterminated triple-quoted string literal (detected at line 2)"))),
        ("s = '''abc\n", Some((1, "unterminated triple-quoted string literal (detected at line 1)"))),
        ("x = {\n1: 2,\ndef f(): pass\n", Some((1, "'{' was never closed"))),
        ("[1,\n2 3]\n", Some((2, "invalid syntax. Perhaps you forgot a comma?"))),
        ("(a\nb)\n", Some((1, "invalid syntax. Perhaps you forgot a comma?"))),
        ("f(a b\n", Some((1, "'(' was never closed"))),
        ("x = (1 2\n", Some((1, "'(' was never closed"))),
        ("try:\n  pass\nelse:\n  pass\n", Some((3, "expected 'except' or 'finally' block"))),
        ("\tif 1:\n        x\n\ty\n", Some((1, "unexpected indent"))),
    ];
    let mut wrong = Vec::new();
    for (source, want) in cases {
        let got = syntax_error(source).map(|e| (e.line, e.msg));
        if got.as_ref().map(|(l, m)| (*l, m.as_str())) != *want {
            wrong.push(format!("{source:?}: CPython {want:?}, engine {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
