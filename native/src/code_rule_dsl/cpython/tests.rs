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

/// The sha256 of `ast.dump(node, include_attributes=False)` of an expression statement's value, as CPython 3.13.14
/// gives it (W's `dump_sha256`): fields left out where `None` or empty, constants' and identifiers' reprs.
#[test]
fn dump_is_cpythons() {
    use super::ast::{F, module};
    use super::dump::Dump;
    let cases: &[(&str, &str)] = &[
        (r#"a or b or not c"#, "ce6a13ccbffccb13a0ef3e7089a57154cafbc7e6bebf1fe06eec8b867b0c7141"),
        (r#"a and (b or c)"#, "227fcda4be63ca3480469961a059dadc1f9a38d6fc51fccbbb445844875da93f"),
        (r#"not (a and b)"#, "378d7098be350643af4759a37f8cce46926b3278bc5ef385ce6543dc2b18bd2d"),
        (r#"-x ** 2"#, "c966ea823e647742e0d0c38e6cd6bd05eb2e6ed217bcb21629b97ee707fc3c7e"),
        (r#"(-x) ** 2"#, "30d5b00adfb8fb7e73f311277ac6426e31fe97b45f0333c72c43f94098fe2ac6"),
        (r#"x ** -y"#, "8c100a5370fcaed83fddce6c9d9701e9d697ed90fc4a145588c674d9bf221cbe"),
        (r#"a - (b - c)"#, "07b10761fafcd394278147dd0b44736cb05a18922c2d75cd057f8ffb4f5fc89e"),
        (r#"(a - b) - c"#, "4fa3f248892eea0a30d04de1f1a5ed808fc37705c607d30a088e61a3bb979cac"),
        (r#"a if b else c if d else e"#, "cdbf2f7c39942f4ab73d8c7c26a99d65b57df947472407e312a167ad1cf08e9a"),
        (r#"(a if b else c) if d else e"#, "76a60e7bcbcd5cdea040ac99dcef0b0b96f293d021640c2623fc48c2a4565027"),
        (r#"lambda: 0"#, "ec9acbea21c9bdccb4b83e0f387a32f7e0b490f0771da75edcb9d388874200d0"),
        (r#"lambda x, /, y=1, *a, z, **k: x"#, "d9e5de42b84ad3876f8a465881c03659f1bee77491ea12044586853a814618cd"),
        (r#"lambda *, k=2: k"#, "595ba6c99f02775c61af0fe70760b219e58e67e22fc616cc901d4bdf7ab2b579"),
        (r#"(yield)"#, "2f5cbc5542dfba1332aa4db3b30475074906fb2520efc1b7d133e6e17716406c"),
        (r#"(yield x + 1)"#, "e13768f372814781ccefa54725ff9c166748bb5d25d3de0fb6b5319060c76efd"),
        (r#"(x := a + b)"#, "59e71585fecfc339a4d2f76588ca4c5cebb110e2a22e09d7e3efe412a1a47eb4"),
        (r#"await f(x)"#, "0dbe68bf486877dcd553d927dc13ac03303b38c9a02eb914ac32f220174453b7"),
        (r#"f(*a, **k, x=1)"#, "599cfd5818f9c6fdd4b2946bc90ef8a280ecfdfd7f6f41ac1cc6ab547a8e45de"),
        (r#"f(x for x in y)"#, "149878fed60186827e231372b1d4b163d84ad62264707a09207da60ffabb0dae"),
        (r#"[x async for x in y if x if not z]"#, "138cb4b088ef451bb2c237949e1bee86715b8299eb2814f4cb0f5f097c0f8e55"),
        (r#"{k: v for k, v in d.items()}"#, "01d34519af5b3e14493943d21352941989fda9b00e6bb9c560bf0779add31c2b"),
        (r#"{**a, 'b': 1}"#, "0227a34821bbe92e2d07a36d251b4d40dee1df3dcef926c524c76c5cabdee15d"),
        (r#"{*()}"#, "0b5f920743205c2f9b1733a9e2f017dbc62bff0ee2408de39393a2115146ec53"),
        (r#"set()"#, "838cdcc98951e8453a6f9eabf156b2a00af54241b1675173b0420d161d1cb68b"),
        (r#"()"#, "f217ece579965832bd632d83e033cabe4f0e799efdf7da507335462b328c704e"),
        (r#"(1,)"#, "7a8490f0ec1df76a90d19f09a52c88f29ca443a7d321ea7b0d23b0fe15907b96"),
        (r#"1, 2"#, "2e734fe79922a3dd923bfc474f44196e8994f863bc09f0d974a723fab40039bb"),
        (r#"a[1:2, ::3]"#, "182bf86973ecbb01e9d4c91cd06ab36e6ed3eb0479456fc2e87fe7db0f99872e"),
        (r#"a[1:2]"#, "17928b51680451ff7ccf03a68d51603d159290122703574a955f760d8a2e2f5e"),
        (r#"a[()]"#, "f0f8ab47d3dc51adc917b15d6d2b1b3b38dd4f5332dd3ce6656f52cc7de98975"),
        (r#"a[(1, 2)]"#, "30b33da55e1551db021e25e06831666b4b4fa2c8ba5e20a9e70de8350b97aaca"),
        (r#"a[*b]"#, "2b40e6bbcd9a85cfb09e115fccd52909ddc6dfa3e3850899496d70b75c9ddf5e"),
        (r#"1 .real"#, "1b30e626eb1541311da102c2197966f6afa1847a52fe68f094d63e6f23bfa8c0"),
        (r#"True.real"#, "bb666dda6a3f65468961c0bb883ea5c0fb49050403c8c28021fd4f46833dc316"),
        (r#"1.5.real"#, "3589614876d61fbcf05ae6fc946716e6c94f933b936575608faefe9f16dce304"),
        (r#"x.y.z"#, "f010430dc31ba05a3452617c434e00f8bb4fbb28173703503671b81013826884"),
        (r#"'it''s'"#, "6cdf541d404c96a16f9c1168f9da2a29c728365d3c1ae9af8ed9f54c2bf59d9c"),
        (r#"'a"b' "c'd""#, "6e64fcb6fc034e146eba4cbaf839689e73fc74ef264cba0d4929d420fbc4feee"),
        (r#"b'\x00\'"'"#, "576cfea1c340526c321e01363a5abc9033b129ee415d994e2f8d1d28fd95b218"),
        (r#"u'x'"#, "077e6299c891c62b6fabc0e3be026608f0ad957c0a29043b41fa18810cd03320"),
        (r#"1e400"#, "30db0b49fa8d681a2aadce9f5a18e075d40155ab2255d894f96858ef5c871e26"),
        (r#"1e400j"#, "554c5f5d3cf2b59774116b6cde240dd8de0d77a811e381820d806bd305b4f2e1"),
        (r#"2j"#, "7b257bd8d5c5284d7f2e9573d55559a4213b3f414ffc9de5e052beca7b183f4b"),
        (r#"0x1F"#, "61b93ad8221122827735930a36fea7245a54891d7600c9ea44f867c1ccf9805e"),
        (r#"1_000_000"#, "ef8f85c6198d4c87f4ee0a5313eb7de196dfa82296503d738fd34c1606ed04f9"),
        (r#"..."#, "ae8f32274f0da1137ff879529be3db36bc7f2194695a0ca0caf6fb9cf558d29d"),
        (r#"f'{x}'"#, "0fcb1db23ccc196ad33fe2625c5aacbc487043da5a445f37fafeda2a569573e8"),
        (r#"f'{x!r:{y}.{z}}'"#, "d75d2908e49a1e935f63506bce1bde7b7bb6888f3494d0b98651d37327311324"),
        (r#"f'{ {1: 2}[1]}'"#, "a83e7358649e756334c481b5bf4995fba979c3788d81340ded47536dbcc0c4a9"),
        (r#"f"{'a'}""#, "768339120c26315c65979928c3921ff9d334a8ff691f6ed980b96b8ccee89659"),
        (r#"f'''{"a"}{'b'}'''"#, "c2ab21b6b2db43d5e185715e6656690a3cd3d43c78ad4c1cc7afed76213d60d3"),
        (r#"f'\n{x}'"#, "ef26abaef9f25f1c9156cff637ab7da45ec8d76c704a0a142c2453d37eefa455"),
        (r#"f'{x:\n}'"#, "60de4e7786edf805d63ab72650370fcd4680318bec1771c0be2dafc1293269b2"),
        (r#"f'a{{b}}c'"#, "3f7c70ded5e76232ee3c5ab5ebb84df477f622a17406deb3541552d4cb6ef1a9"),
        (r#"f'{x=!s:>4}'"#, "b9121455a2ee41c502a292c9a676575d914e5d2fa58f6cf2afa3b2ccc788cd08"),
        (r#"f"'" f'"'"#, "275a3357fa45dd0ea3e990238c97ab1569dcc6b5ebbc04ee132c8f107d1fc822"),
        (r#"'\t\x7f\u200b\U0001f600é'"#, "2a7520260aec5597ab0eb3c2ddf1e0815689c209f6446f8b6e4b5a6896298555"),
        (r#"a < b < c"#, "6156a030c8712679a075dfc4dae06ae8b7c7b33f6c0ac15953116e87751f8820"),
        (r#"a not in b is not c"#, "0668330baeff9033ac751abe983cbd058cd6063d0a1c4c833b1a2aa2f08abfde"),
        (r#"*a, b"#, "466d4d9d533d0e6506d78a725712be9c6109f3de1b0e63016e00a8bd6b07f877"),
        (r#"a @ b // c % d"#, "9436286ee0c6b1041b8e432a40c9d5e7ea880af0beca4bec1d2c061d86335d2a"),
        (r#"a | b ^ c & d << e"#, "388fb0155abd18009cf4a444b5becf06100ef2952802dcab5b5a0b223c697126"),
        (r#"~a"#, "7849aef2899b0aa9a3e8e2b66dd11f7aec759b0e5b2b1e8be6326404d65db403"),
        (r#"+a"#, "7d495c008737dd2a54282b5079485b4f6ab8fc6a54e44d4f0dd7201bdc71671a"),
        (r#"(lambda: 1)()"#, "0e3b0aabe0963fd2d0a39b30dee3c8b91260c300b01ad0d3cbdf32022b670579"),
        (r#"(a or b).c"#, "6bb62e2d873a1bc8c76873c27532947e3ca602761747fb0f01b33caad80a16ac"),
        (r#"x[a or b]"#, "0c5c91d9895de2570bb43a4ffde9d57072af115d26f9c5bba12203c077bbe6e7"),
        (r#"f'{(yield)}'"#, "1113f0a08daaf41c8513fc06db5bd110b5461b29fd15491cd559aaa5df14a173"),
        (r#"f'{(lambda: 1)}'"#, "d5fc0296044bff297693850febd7f655e15774c0bfc682dcd81a4ee40143e9e8"),
        (r#"[*a, *b]"#, "52f537ac8ce0ffb09dd4f4c056959d864d989c58e74e87e74472dd7fae6613d9"),
        (r#"print(*(a, b))"#, "f275c25d368ce0c42d5ef4c8c27e844592b4a848c5b678aa583d367ab6b5d33a"),
        (r#"{1, 2}"#, "7278e1239e592ff1756503ba1d32f6539041e52125053030bd105b998b877cb0"),
        (r#"a if (b := c) else d"#, "0f9d24719ddb6fbac4d66ccf415650db1808aa90112215ea5d229ea446daaefb"),
        (r#"'\ud800'"#, "cbe17708ec00aea54a34e395dbb59ddb76657198fe19ed002a16a63b3a7ac6bc"),
        (r#"f'{x!a}' 'tail\'s'"#, "dc345a94fc186d49976a96b453dfeab4aee396b9c3f64ea14ef71ee6a6ac65b6"),
        (r#"f'''a
b{x}'''"#, "b10b63fb5ab09d3c9794cfe157dab54afcb2debf26c6ae5f94baa4cd1f85aa44"),
        (r#"None"#, "8f2cda988d9ca73053c7f508ff6d429ab1df0a26a6352507088f780e596b398c"),
        (r#"True"#, "fc49e3ff31db631b11fdae65c1243eea50afe64ff7f1cddca82c434c55e40498"),
        (r#"-1"#, "dec7d1986baf996d5c6473b741af733a83dd791ce21c6d2c728aa85f20720e73"),
        (r#"1.0"#, "a77ce679a39d58612d22d1de65d61b51b39b980ebe20eded1309aa6215eb9387"),
        (r#"0.1"#, "a270ddb9e0765184c77aa05a603b2fabfa8bb4420e1be9dfbfd1177918388c36"),
        (r#"1e16"#, "8e0fc14fb0616a620c8c10df0b295a27c5a7d9cb941805a9008da0506343fe53"),
        (r#"1e-5"#, "546257d8e6f617d5bc2140153c93693d746ad2566e913cfe372b3a9f8ac6cd91"),
        (r#"x[1:2:3]"#, "2b3f62f1dcdf7697b86157b8bb6d646f6ea5b145dadd9f0c1dab4bb273f732d2"),
        (r#"x[:]"#, "b8aa7a8f9f99631d85a4a7702609299510f463b4e4fe69f1ab4edb5f5fd2191b"),
        (r#"f(a)(b)[c].d"#, "7e6857e83301784a089dae2640c94225dd218a7b1d64870a046d47fadd8f0927"),
        (r#"(a, *b) if c else ()"#, "d0ec4fd0c20bcea283594e6b76330c857b765ec35ea37f2b2d743e258753f991"),
    ];
    for &(source, sha) in cases {
        let root = module(source).unwrap_or_else(|| panic!("{source:?} parses"));
        let Some(F::Node(stmt)) = root.list("body").first() else { panic!("{source:?} has a statement") };
        let value = stmt.node("value").unwrap_or_else(|| panic!("{source:?} is an expression"));
        assert_eq!(Dump::of(&root).sha256(value).as_deref(), Some(sha), "dump of {source:?}");
    }
}

/// W's `body_sha256` and `signature_sha256` of a def, and the text each hashes, as CPython 3.13.14 gives them.
#[test]
fn def_hashes_are_ws() {
    use super::ast::{F, module};
    use super::dump::Dump;
    let source = "def f(a: \"x\", *b: int, c=1) -> dict[str, int]:\n    return a\n";
    let root = module(source).unwrap();
    let Some(F::Node(def)) = root.list("body").first() else { panic!("a def") };
    let dump = Dump::of(&root);
    assert_eq!(
        dump.signature_text(def).as_deref(),
        Some(
            "arguments(args=[arg(arg='a', annotation=Constant(value='x'))], vararg=arg(arg='b', annotation=Name(id='int', ctx=Load())), \
             kwonlyargs=[arg(arg='c')], kw_defaults=[Constant(value=1)])|Subscript(value=Name(id='dict', ctx=Load()), \
             slice=Tuple(elts=[Name(id='str', ctx=Load()), Name(id='int', ctx=Load())], ctx=Load()), ctx=Load())|Tuple(ctx=Load())"
        )
    );
    assert_eq!(dump.body_text(def).as_deref(), Some("Module(body=[Return(value=Name(id='a', ctx=Load()))])"));
    assert_eq!(dump.body_sha256(def).as_deref(), Some("747bc3749caaa6d222424d1542da2ee0a312fe508fb3081c65d873c88311bae1"));
    assert_eq!(dump.signature_sha256(def).as_deref(), Some("f162c45775dc52a80bbf80593192690791f38ed2ee5db47bdd0582cf37943108"));
    assert_eq!(dump.sha256(def).as_deref(), Some("86250c1f242392eacdb303a6c1df981f77a09a769fdd196f207f0e2ec43ae458"));
}
