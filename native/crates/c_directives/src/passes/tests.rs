//! The programs against the serial pass: the spike's cases (`rerail_c_pp::stream::parallel`'s
//! tests), each unit read by the programs on the interpreter and compared token by token.

use super::*;
use rerail_c_pp::files::{DirKind, Search};
use rerail_c_pp::macros::Tk;
use rerail_c_pp::sink::{At, Sink};
use rerail_c_pp::{Options, Pp};

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

/// `main.c` of `files` serially and by the programs: the programs' outcome, checked equal to
/// the serial pass where they read it.
fn both(files: &[(&str, &str)]) -> (Outcome, Stats) {
    let cfg = Config::new(Options::c(17, true));
    let mut serial = Spellings::default();
    let mut s_io = io(files);
    let mut pp = Pp::new(cfg.clone(), &mut s_io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let (out, stats) = run(cfg, &mut io(files), "main.c");
    if let Ok(o) = &out {
        assert_eq!(show(o), show(&serial.0), "the programs against the serial pass");
    }
    (out, stats)
}

fn same_in(files: &[(&str, &str)]) -> Stats {
    let (out, stats) = both(files);
    if let Err(why) = out {
        panic!("refused: {why}");
    }
    stats
}

fn same(src: &str) -> Stats {
    same_in(&[("main.c", src)])
}

#[test]
fn the_programs_verify_and_read_back_from_their_text() {
    for prog in [&*ROUND, &*TEXT] {
        let mut w = rerail_compiler::program::Words::default();
        w.words = vec![rerail_compiler::program::word(rerail_compiler::program::STOP, 0, 0)];
        w.array_programs.push(prog.p.clone());
        let text = rerail_compiler::rir_asm::disassemble(&w);
        let again = rerail_compiler::rir_asm::assemble(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.array_programs[0], prog.p);
    }
}

#[test]
fn macros_object_and_function_like() {
    same("#define N 3\n#define SQ(x) ((x)*(x))\nint a = N + SQ(N + 1);\nint b = SQ(SQ(2));\n");
    same("#define f(a, ...) g(a __VA_OPT__(,) __VA_ARGS__)\nf(1) f(1, 2, 3) f()\n");
    same("#define str(s) # s\n#define xstr(s) str(s)\n#define INCFILE(n) vers ## n\nxstr(INCFILE(2).h) str(strncmp(\"abc\\0d\", \"abc\", '\\4') == 0)\n");
}

#[test]
fn visibility_follows_position() {
    same("#define A 1\nA\n#undef A\nA\n#define A 2\nA\n#define B A\n#undef A\nB\n#define A 3\nB\n");
}

#[test]
fn conditionals_nested_and_chained() {
    same("#define X 2\n#if X == 1\none\n#elif X == 2\ntwo\n#if defined(Y)\ny\n#else\nnot_y\n#endif\n#else\nother\n#endif\nend\n");
    same("#ifdef A\na\n#elifndef B\nnot_b\n#endif\n#ifndef C\n#define C 1\n#endif\n#if C\nc\n#endif\n");
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
    same_in(&files);
}

#[test]
fn an_include_named_by_a_macro_and_has_include() {
    same_in(&[("main.c", "#define H \"h.h\"\n#include H\n#if __has_include(\"h.h\") && !__has_include(\"no.h\")\nyes\n#endif\nV\n"), ("h.h", "#define V 7\n")]);
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

#[test]
fn counter_line_and_file() {
    same("#define C __COUNTER__\nC C\n#define two(a, b) a b\ntwo(__COUNTER__, C) __COUNTER__\n");
    same("#define L __LINE__\nL\n#define call(a, b) a b\ncall(L,\n  __LINE__)\n__FILE__ __INCLUDE_LEVEL__\n");
    same_in(&[("main.c", "#include \"i.h\"\n__INCLUDE_LEVEL__\n"), ("i.h", "__FILE__ __INCLUDE_LEVEL__ __LINE__\n")]);
}

#[test]
fn pushes_and_pops() {
    same("#define A 1\n#pragma push_macro(\"A\")\n#undef A\nA\n#define A 2\nA\n#pragma pop_macro(\"A\")\nA\n");
    same("#define min(a, b) m\n#define PUSH _Pragma(\"push_macro(\\\"min\\\")\") _Pragma(\"push_macro(\\\"B\\\")\")\n#define POP _Pragma(\"pop_macro(\\\"min\\\")\") _Pragma(\"pop_macro(\\\"B\\\")\")\nPUSH\n#undef min\nmin(1, 2)\n#if defined(min)\nbad\n#endif\nPOP\nmin(1, 2)\n#ifdef min\nback\n#endif\n");
    same_in(&[("main.c", "#include \"o.h\"\n#include \"o.h\"\nO\n"), ("o.h", "_Pragma(\"once\")\n#define O o\nbody\n")]);
}

/// What the programs do not carry yet is refused, with its reason: merges (none in either
/// corpus), a pop with no push, and what the spike refuses.
#[test]
fn what_is_refused() {
    let refused = |src: &str| both(&[("main.c", src)]).0.err();
    assert_eq!(refused("#define f(x) g\n#define g(y) y + 1\nf(1)(2)\n"), Some("a merge"));
    assert_eq!(refused("#define h(a) [a]\n#define H h(\nH 5) end\n"), Some("a merge"));
    assert_eq!(refused("#define f(x) g\n#define g(y) y\nf(1) + 2\n"), None);
    assert_eq!(refused("#define A 1\n#pragma pop_macro(\"A\")\nA\n"), Some("a pop_macro with no push"));
    assert_eq!(refused("#line 10\n__LINE__\n"), Some("#line"));
    assert_eq!(refused("#if __has_feature(x\nyes\n#endif\nend\n"), None);
    assert_eq!(refused("#define f(a, b) a b\nf(1,\n#ifdef X\n2\n#else\n3\n#endif\n)\n"), Some("a call past its stretch of text"));
    assert_eq!(refused("#if 0\n#line 10\n#endif\nok\n"), None);
}
