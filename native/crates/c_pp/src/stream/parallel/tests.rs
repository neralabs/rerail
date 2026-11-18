use super::super::Pp;
use super::*;
use crate::files::{DirKind, Search};
use crate::macros::Tk;
use crate::sink::{At, Sink};
use crate::Options;
use std::collections::HashMap;

#[derive(Default)]
struct Spellings(Vec<Vec<u8>>);

impl Sink for Spellings {
    fn token(&mut self, _t: &Tk, text: &[u8], _at: Option<At>, _off: u32) {
        self.0.push(text.to_vec());
    }
}

fn io(files: &[(&str, &str)]) -> Search<impl FnMut(&str) -> Option<Vec<u8>>> {
    let m: HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    Search::new(vec![("sys".into(), DirKind::System)], move |p: &str| m.get(p).cloned())
}

fn show(v: &[Vec<u8>]) -> String {
    v.iter().map(|t| String::from_utf8_lossy(t).into_owned()).collect::<Vec<_>>().join(" ")
}

/// `main.c` of `files`, serially and as passes: the passes' reading, which the test checks is
/// the serial one's, and its shape.
fn both(files: &[(&str, &str)], cfg: Config) -> (Outcome, Stats) {
    let mut serial = Spellings::default();
    let mut s_io = io(files);
    let mut pp = Pp::new(cfg.clone(), &mut s_io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let (out, stats) = run(cfg, &mut io(files), "main.c");
    if let Ok(o) = &out {
        assert_eq!(show(o), show(&serial.0), "the passes against the serial pass");
    }
    (out, stats)
}

fn c() -> Config {
    Config::new(Options::c(17, true))
}

/// `main.c` alone is read as passes, as the serial pass reads it.
fn same(src: &str) -> Stats {
    same_in(&[("main.c", src)])
}

fn same_in(files: &[(&str, &str)]) -> Stats {
    let (out, stats) = both(files, c());
    if let Err(why) = out {
        panic!("sent to the serial pass: {why}");
    }
    stats
}

#[test]
fn macros_object_and_function_like() {
    same("#define N 3\n#define SQ(x) ((x)*(x))\nint a = N + SQ(N + 1);\nint b = SQ(SQ(2));\n");
    same("#define f(a, ...) g(a __VA_OPT__(,) __VA_ARGS__)\nf(1) f(1, 2, 3) f()\n");
    same("#define str(s) # s\n#define xstr(s) str(s)\n#define INCFILE(n) vers ## n\nxstr(INCFILE(2).h) str(strncmp(\"abc\\0d\", \"abc\", '\\4') == 0)\n");
}

/// C17 §6.10.3.5's examples 3 and 4: rescanning, painting, and `#`/`##`.
#[test]
fn the_standards_examples() {
    same(
        "#define x 3\n#define f(a) f(x * (a))\n#undef x\n#define x 2\n#define g f\n#define z z[0]\n#define h g(~\n#define m(a) a(w)\n#define w 0,1\n#define t(a) a\n#define p() int\n#define q(x) x\n#define r(x,y) x ## y\n#define str(x) # x\n\
         f(y+1) + f(f(z)) % t(t(g)(0) + t)(1);\ng(x+(3,4)-w) | h 5) & m\n(f)^m(m);\np() i[q()] = { q(1), r(2,3), r(4,), r(,5), r(,) };\nchar c[2][6] = { str(hello), str() };\n",
    );
}

#[test]
fn visibility_follows_position() {
    let st = same("#define A 1\nA\n#undef A\nA\n#define A 2\nA\n#define B A\n#undef A\nB\n#define A 3\nB\n");
    assert_eq!(st.definitions, 6);
}

#[test]
fn conditionals_nested_and_chained() {
    same("#define X 2\n#if X == 1\none\n#elif X == 2\ntwo\n#if defined(Y)\ny\n#else\nnot_y\n#endif\n#else\nother\n#endif\nend\n");
    same("#ifdef A\na\n#elifndef B\nnot_b\n#endif\n#ifndef C\n#define C 1\n#endif\n#if C\nc\n#endif\n");
    // A test that depends on a definition made in a group: two rounds' reading at least.
    let st = same("#ifndef CFG\n#define CFG 1\n#endif\n#if CFG\n#define ON 1\n#endif\n#ifdef ON\non\n#endif\n");
    assert!(st.rounds >= 2, "{st:?}");
}

#[test]
fn includes_guards_and_once() {
    let files = [
        ("main.c", "#include \"a.h\"\n#include \"a.h\"\n#include \"o.h\"\n#include \"o.h\"\n#include <s.h>\nA O S\n"),
        ("a.h", "#ifndef A_H\n#define A_H\n#define A from_a\na_body\n#endif\n"),
        ("o.h", "#pragma once\n#define O from_o\no_body\n"),
        ("sys/s.h", "#define S from_s\n#include \"a.h\"\n"),
    ];
    let st = same_in(&files);
    // The predefines, main.c, a.h twice (the second's group not taken), o.h once, sys/s.h (its
    // "a.h" is no file next to it nor in the search path).
    assert_eq!(st.instances, 6, "{st:?}");
}

#[test]
fn an_include_named_by_a_macro_and_has_include() {
    let files = [
        ("main.c", "#define H \"h.h\"\n#include H\n#if __has_include(\"h.h\") && !__has_include(\"no.h\")\nyes\n#endif\nV\n"),
        ("h.h", "#define V 7\n"),
    ];
    same_in(&files);
}

#[test]
fn an_include_switched_by_an_earlier_one() {
    let files = [
        ("main.c", "#include \"cfg.h\"\n#if USE_B\n#include \"b.h\"\n#else\n#include \"a.h\"\n#endif\nWHICH\n"),
        ("cfg.h", "#define USE_B 1\n"),
        ("a.h", "#define WHICH a\n"),
        ("b.h", "#define WHICH b\n"),
    ];
    same_in(&files);
}

/// An expansion that ends in a function-like name takes the call after it.
#[test]
fn merges_past_an_invocations_end() {
    let st = same("#define f(x) g\n#define g(y) y + 1\n#define F f\nf(1)(2) F(3)(4)\n");
    assert!(st.merges >= 2, "{st:?}");
    // A call opened by an expansion, its arguments in the text after it.
    let st = same("#define h(a) [a]\n#define H h(\nH 5) end\n");
    assert!(st.merges >= 1, "{st:?}");
    // A function-like name with no call after it.
    same("#define f(x) g\n#define g(y) y\nf(1) + 2\n");
}

#[test]
fn counter_line_and_file() {
    same("#define C __COUNTER__\nC C\n#define two(a, b) a b\ntwo(__COUNTER__, C) __COUNTER__\n");
    same("#define L __LINE__\nL\n#define call(a, b) a b\ncall(L,\n  __LINE__)\n__FILE__ __INCLUDE_LEVEL__\n");
    let files = [("main.c", "#include \"i.h\"\n__INCLUDE_LEVEL__\n"), ("i.h", "__FILE__ __INCLUDE_LEVEL__ __LINE__\n")];
    same_in(&files);
}

/// What the shape does not carry goes to the serial pass, with its reason.
#[test]
fn what_goes_to_the_serial_pass() {
    let refused = |src: &str| both(&[("main.c", src)], c()).0.err();
    assert_eq!(refused("#define A 1\n#pragma push_macro(\"A\")\n#undef A\n#pragma pop_macro(\"A\")\nA\n"), Some("push_macro or pop_macro"));
    assert_eq!(refused("#line 10\n__LINE__\n"), Some("#line"));
    assert_eq!(refused("#define f(a, b) a b\nf(1,\n#ifdef X\n2\n#else\n3\n#endif\n)\n"), Some("a call past its stretch of text"));
    // An unsupported directive in a group not taken is nothing.
    assert_eq!(refused("#if 0\n#line 10\n#endif\nok\n"), None);
}
