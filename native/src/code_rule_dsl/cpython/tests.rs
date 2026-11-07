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

/// `literal_json`: `json.dumps(ast.literal_eval(e), sort_keys=True, separators=(",", ":"))` as CPython 3.13.18 gives it
/// (`python3.13 -I -S`, on nera-port-4), `None` where it raises: ints of every base and size, floats as
/// `float.__repr__`, escapes, a lone surrogate, sets, bytes and complex numbers, merged and sorted dict keys.
#[test]
fn literal_json_is_cpythons() {
    let cases: &[(&str, Option<&str>)] = &[
        ("1", Some("1")),
        ("-1", Some("-1")),
        ("+1", Some("1")),
        ("-0", Some("0")),
        ("1_000_000", Some("1000000")),
        ("0x_ff", Some("255")),
        ("0o17", Some("15")),
        ("0b101", Some("5")),
        ("0xFFFFFFFFFFFFFFFFFFFFFFFF", Some("79228162514264337593543950335")),
        ("-0x8000000000000000", Some("-9223372036854775808")),
        ("1.5", Some("1.5")),
        ("-0.0", Some("-0.0")),
        ("0.1", Some("0.1")),
        ("1e16", Some("1e+16")),
        ("1e15", Some("1000000000000000.0")),
        ("1e-5", Some("1e-05")),
        ("0.0001", Some("0.0001")),
        ("1e999", Some("Infinity")),
        ("-1e999", Some("-Infinity")),
        ("123456789012345678.0", Some("1.2345678901234568e+17")),
        ("2.5e-300", Some("2.5e-300")),
        ("1.0", Some("1.0")),
        ("3.14159265358979", Some("3.14159265358979")),
        ("True", Some("true")),
        ("False", Some("false")),
        ("None", Some("null")),
        ("...", None),
        ("'a'", Some("\"a\"")),
        ("'\u{e9}'", Some("\"\\u00e9\"")),
        ("'\\U0001f600'", Some("\"\\ud83d\\ude00\"")),
        ("'\\x7f\\x00\\x1f'", Some("\"\\u007f\\u0000\\u001f\"")),
        ("'\\ud800'", Some("\"\\ud800\"")),
        ("'a' 'b'", Some("\"ab\"")),
        ("'\"\\\\'", Some("\"\\\"\\\\\"")),
        ("b'x'", None),
        ("1j", None),
        ("1+2j", None),
        ("-1-2j", None),
        ("(1, 2)", Some("[1,2]")),
        ("()", Some("[]")),
        ("[1, [2, (3,)]]", Some("[1,[2,[3]]]")),
        ("{1, 2}", None),
        ("set()", None),
        ("{}", Some("{}")),
        ("{'b': 1, 'a': 2}", Some("{\"a\":2,\"b\":1}")),
        ("{1: 'a', True: 'b'}", Some("{\"1\":\"b\"}")),
        ("{1: 'a', 1.0: 'c', 2: 'd'}", Some("{\"1\":\"c\",\"2\":\"d\"}")),
        ("{'a': 1, 2: 3}", None),
        ("{None: 1}", Some("{\"null\":1}")),
        ("{None: 1, 'a': 2}", None),
        ("{(1,): 2}", None),
        ("{1: b'x', 1: 2}", Some("{\"1\":2}")),
        ("{2.5: 1, 1: 2, False: 3}", Some("{\"false\":3,\"1\":2,\"2.5\":1}")),
        ("{[1]: 2}", None),
        ("{1: {1, [2]}}", None),
        ("{1: {'z': 1, 'y': [None, True]}}", Some("{\"1\":{\"y\":[null,true],\"z\":1}}")),
        ("{**x}", None),
        ("[x]", None),
        ("f'a'", None),
        ("-True", None),
        ("--1", None),
        ("~1", None),
        ("not 1", None),
        ("{1: (1+0j)}", None),
        ("{1: 'a', (1+0j): 'b'}", Some("{\"1\":\"b\"}")),
        ("'\\N{BULLET}'", Some("\"\\u2022\"")),
        ("r'\\ud800'", Some("\"\\\\ud800\"")),
        ("{'\\U0001f600': 1, '\\uffff': 2}", Some("{\"\\uffff\":2,\"\\ud83d\\ude00\":1}")),
        ("{1e16: 1, 10**2: 2}", None),
        ("[1e300*1e10]", None),
        ("{0.5: 1, 0: 2, -1: 3}", Some("{\"-1\":3,\"0\":2,\"0.5\":1}")),
    ];
    let mut wrong = Vec::new();
    for &(source, want) in cases {
        let got = super::literal::literal_json_of(&format!("{source}\n"));
        if got.as_deref() != want {
            wrong.push(format!("{source:?}: CPython {want:?}, engine {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// `ast.unparse` of an expression statement's value, as CPython 3.13.14 gives it: precedence and its quirks, constants'
/// reprs, f-strings' quotes.
#[test]
fn unparse_is_cpythons() {
    use super::ast::{F, module};
    let cases: &[(&str, &str)] = &[
        (r#"a or b or not c"#, r#"a or b or (not c)"#),
        (r#"a and (b or c)"#, r#"a and (b or c)"#),
        (r#"not (a and b)"#, r#"not (a and b)"#),
        (r#"-x ** 2"#, r#"-x ** 2"#),
        (r#"(-x) ** 2"#, r#"(-x) ** 2"#),
        (r#"x ** -y"#, r#"x ** (-y)"#),
        (r#"a - (b - c)"#, r#"a - (b - c)"#),
        (r#"(a - b) - c"#, r#"a - b - c"#),
        (r#"a if b else c if d else e"#, r#"a if b else c if d else e"#),
        (r#"(a if b else c) if d else e"#, r#"(a if b else c) if d else e"#),
        (r#"lambda: 0"#, r#"lambda: 0"#),
        (r#"lambda x, /, y=1, *a, z, **k: x"#, r#"lambda x, /, y=1, *a, z, **k: x"#),
        (r#"lambda *, k=2: k"#, r#"lambda *, k=2: k"#),
        (r#"(yield)"#, r#"(yield)"#),
        (r#"(yield x + 1)"#, r#"(yield (x + 1))"#),
        (r#"(x := a + b)"#, r#"(x := (a + b))"#),
        (r#"await f(x)"#, r#"await f(x)"#),
        (r#"f(*a, **k, x=1)"#, r#"f(*a, **k, x=1)"#),
        (r#"f(x for x in y)"#, r#"f((x for x in y))"#),
        (r#"[x async for x in y if x if not z]"#, r#"[x async for x in y if x if not z]"#),
        (r#"{k: v for k, v in d.items()}"#, r#"{k: v for k, v in d.items()}"#),
        (r#"{**a, 'b': 1}"#, r#"{**a, 'b': 1}"#),
        (r#"{*()}"#, r#"{*()}"#),
        (r#"set()"#, r#"set()"#),
        (r#"()"#, r#"()"#),
        (r#"(1,)"#, r#"(1,)"#),
        (r#"1, 2"#, r#"(1, 2)"#),
        (r#"a[1:2, ::3]"#, r#"a[1:2, ::3]"#),
        (r#"a[1:2]"#, r#"a[1:2]"#),
        (r#"a[()]"#, r#"a[()]"#),
        (r#"a[(1, 2)]"#, r#"a[1, 2]"#),
        (r#"a[*b]"#, r#"a[*b,]"#),
        (r#"1 .real"#, r#"1 .real"#),
        (r#"True.real"#, r#"True .real"#),
        (r#"1.5.real"#, r#"1.5.real"#),
        (r#"x.y.z"#, r#"x.y.z"#),
        (r#"'it''s'"#, r#"'its'"#),
        (r#"'a"b' "c'd""#, r#"'a"bc\'d'"#),
        (r#"b'\x00\'"'"#, r#"b'\x00\'"'"#),
        (r#"u'x'"#, r#"u'x'"#),
        (r#"1e400"#, r#"1e309"#),
        (r#"1e400j"#, r#"1e309j"#),
        (r#"2j"#, r#"2j"#),
        (r#"0x1F"#, r#"31"#),
        (r#"1_000_000"#, r#"1000000"#),
        (r#"..."#, r#"..."#),
        (r#"f'{x}'"#, r#"f'{x}'"#),
        (r#"f'{x!r:{y}.{z}}'"#, r#"f'{x!r:{y}.{z}}'"#),
        (r#"f'{ {1: 2}[1]}'"#, r#"f'{ {1: 2}[1]}'"#),
        (r#"f"{'a'}""#, r#"f"{'a'}""#),
        (r#"f'''{"a"}{'b'}'''"#, r#"f"{'a'}{'b'}""#),
        (r#"f'\n{x}'"#, r#"f'\n{x}'"#),
        (r#"f'{x:\n}'"#, r#"f'{x:\n}'"#),
        (r#"f'a{{b}}c'"#, r#"f'a{{b}}c'"#),
        (r#"f'{x=!s:>4}'"#, r#"f'x={x!s:>4}'"#),
        (r#"f"'" f'"'"#, r#"f''''"'''"#),
        (r#"'\t\x7f\u200b\U0001f600é'"#, r#"'\t\x7f\u200b😀é'"#),
        (r#"a < b < c"#, r#"a < b < c"#),
        (r#"a not in b is not c"#, r#"a not in b is not c"#),
        (r#"*a, b"#, r#"(*a, b)"#),
        (r#"a @ b // c % d"#, r#"a @ b // c % d"#),
        (r#"a | b ^ c & d << e"#, r#"a | b ^ c & d << e"#),
        (r#"~a"#, r#"~a"#),
        (r#"+a"#, r#"+a"#),
        (r#"(lambda: 1)()"#, r#"(lambda: 1)()"#),
        (r#"(a or b).c"#, r#"(a or b).c"#),
        (r#"x[a or b]"#, r#"x[a or b]"#),
        (r#"f'{(yield)}'"#, r#"f'{(yield)}'"#),
        (r#"f'{(lambda: 1)}'"#, r#"f'{(lambda: 1)}'"#),
        (r#"[*a, *b]"#, r#"[*a, *b]"#),
        (r#"print(*(a, b))"#, r#"print(*(a, b))"#),
        (r#"{1, 2}"#, r#"{1, 2}"#),
        (r#"a if (b := c) else d"#, r#"a if (b := c) else d"#),
        (r#"'\ud800'"#, r#"'\ud800'"#),
        (r#"f'{x!a}' 'tail\'s'"#, r#"f"{x!a}tail's""#),
        (r#"f'''a
b{x}'''"#, r#"f'a\nb{x}'"#),
        (r#"None"#, r#"None"#),
        (r#"True"#, r#"True"#),
        (r#"-1"#, r#"-1"#),
        (r#"1.0"#, r#"1.0"#),
        (r#"0.1"#, r#"0.1"#),
        (r#"1e16"#, r#"1e+16"#),
        (r#"1e-5"#, r#"1e-05"#),
        (r#"x[1:2:3]"#, r#"x[1:2:3]"#),
        (r#"x[:]"#, r#"x[:]"#),
        (r#"f(a)(b)[c].d"#, r#"f(a)(b)[c].d"#),
        (r#"(a, *b) if c else ()"#, r#"(a, *b) if c else ()"#),
    ];
    for &(source, unparsed) in cases {
        let root = module(source).unwrap_or_else(|| panic!("{source:?} parses"));
        let Some(F::Node(stmt)) = root.list("body").first() else { panic!("{source:?} has a statement") };
        let value = stmt.node("value").unwrap_or_else(|| panic!("{source:?} is an expression"));
        assert_eq!(super::unparse::unparse(value).as_deref(), Some(unparsed), "unparse of {source:?}");
    }
}

