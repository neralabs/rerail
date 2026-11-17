use super::*;
use crate::files::{DirKind, Search};
use crate::sink::{FactSink, Tee};

/// The output tokens' spellings.
#[derive(Default)]
struct Spellings(Vec<String>);

impl Sink for Spellings {
    fn token(&mut self, _t: &Tk, text: &[u8], _at: Option<At>) {
        self.0.push(String::from_utf8_lossy(text).into_owned());
    }
}

fn mem(files: &[(&str, &str)]) -> impl FnMut(&str) -> Option<Vec<u8>> + use<> {
    let m: HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    move |p: &str| m.get(p).cloned()
}

/// `main.c` of `files` preprocessed under `cfg`: its tokens, and its facts.
fn run_with(files: &[(&str, &str)], dirs: &[(&str, DirKind)], cfg: Config) -> (Vec<String>, FactSink) {
    let mut io = Search::new(dirs.iter().map(|(d, k)| (d.to_string(), *k)).collect(), mem(files));
    let (mut toks, mut facts) = (Spellings::default(), FactSink::default());
    let mut tee = Tee(&mut toks, &mut facts);
    let mut pp = Pp::new(cfg, &mut io, &mut tee);
    assert!(pp.run("main.c"));
    drop(pp);
    (toks.0, facts)
}

fn c() -> Config {
    Config::new(Options::c(17, true))
}

fn cpp() -> Config {
    Config::new(Options::cpp(20, true))
}

fn pp(src: &str) -> String {
    pp_in(src, c())
}

fn pp_in(src: &str, cfg: Config) -> String {
    run_with(&[("main.c", src)], &[], cfg).0.join(" ")
}

/// The tokens of `text`, spelled, joined as `pp`'s are.
fn toks(text: &str) -> String {
    let mut lx = PpLexer::new(text.as_bytes(), Options::cpp(20, true));
    let mut v = Vec::new();
    loop {
        let t = lx.next();
        if t.kind == P::Eof {
            break;
        }
        v.push(String::from_utf8_lossy(&text.as_bytes()[t.start as usize..t.end as usize]).into_owned());
    }
    v.join(" ")
}

#[test]
fn c17_example_3_rescanning() {
    let src = r#"#define x 3
#define f(a) f(x * (a))
#undef x
#define x 2
#define g f
#define z z[0]
#define h g(~
#define m(a) a(w)
#define w 0,1
#define t(a) a
#define p() int
#define q(x) x
#define r(x,y) x ## y
#define str(x) # x
f(y+1) + f(f(z)) % t(t(g)(0) + t)(1);
g(x+(3,4)-w) | h 5) & m
(f)^m(m);
p() i[q()] = { q(1), r(2,3), r(4,), r(,5), r(,) };
char c[2][6] = { str(hello), str() };
"#;
    let want = r#"f(2 * (y+1)) + f(2 * (f(2 * (z[0])))) % f(2 * (0)) + t(1);
f(2 * (2+(3,4)-0,1)) | f(2 * (~ 5)) & f(2 * (0,1))^m(0,1);
int i[] = { 1, 23, 4, 5, };
char c[2][6] = { "hello", "" };"#;
    assert_eq!(pp(src), toks(want));
}

#[test]
fn c17_example_4_strings_and_pastes() {
    let src = r#"#define str(s) # s
#define xstr(s) str(s)
#define debug(s, t) printf("x" # s "= %d, x" # t "= %s", \
 x ## s, x ## t)
#define INCFILE(n) vers ## n
#define glue(a, b) a ## b
#define xglue(a, b) glue(a, b)
#define HIGHLOW "hello"
#define LOW LOW ", world"
debug(1, 2);
fputs(str(strncmp("abc\0d", "abc", '\4') // this goes away
 == 0) str(: @\n), s);
#include xstr(INCFILE(2).h)
glue(HIGH, LOW);
xglue(HIGH, LOW)
"#;
    let want = r#"printf("x" "1" "= %d, x" "2" "= %s", x1, x2);
fputs("strncmp(\"abc\\0d\", \"abc\", '\\4') == 0" ": @\n", s);
vers2_h_was_included
"hello";
"hello" ", world""#;
    let (got, facts) = run_with(&[("main.c", src), ("vers2.h", "vers2_h_was_included\n")], &[], c());
    assert_eq!(got.join(" "), toks(want));
    assert_eq!(facts.included, [(1, facts.included[0].1, 2, 1)]);
}

#[test]
fn c17_example_5_placemarkers() {
    let src = "#define t(x,y,z) x ## y ## z\nint j[] = { t(1,2,3), t(,4,5), t(6,,7), t(8,9,),\n t(10,,), t(,11,), t(,,12), t(,,) };\n";
    assert_eq!(pp(src), toks("int j[] = { 123, 45, 67, 89, 10, 11, 12, };"));
}

#[test]
fn c17_example_7_variadic() {
    let src = r#"#define debug(...) fprintf(stderr, __VA_ARGS__)
#define showlist(...) puts(#__VA_ARGS__)
#define report(test, ...) ((test)?puts(#test):\
 printf(__VA_ARGS__))
debug("Flag");
debug("X = %d\n", x);
showlist(The first, second, and third items.);
report(x>y, "x is %d but y is %d", x, y);
"#;
    let want = r#"fprintf(stderr, "Flag");
fprintf(stderr, "X = %d\n", x);
puts("The first, second, and third items.");
((x>y)?puts("x>y"): printf("x is %d but y is %d", x, y));"#;
    assert_eq!(pp(src), toks(want));
}

#[test]
fn cpp20_va_opt() {
    let src = r#"#define F(...) f(0 __VA_OPT__(,) __VA_ARGS__)
#define G(X, ...) f(0, X __VA_OPT__(,) __VA_ARGS__)
#define SDEF(sname, ...) S sname __VA_OPT__(= { __VA_ARGS__ })
#define EMP
F(a,b,c)
F()
F(EMP)
G(a,b,c)
G(a,)
G(a)
SDEF(foo);
SDEF(bar, 1, 2);
"#;
    let want = "f(0, a, b, c) f(0) f(0) f(0, a, b, c) f(0, a) f(0, a) S foo; S bar = { 1, 2 };";
    assert_eq!(pp_in(src, cpp()), toks(want));
    let src = r#"#define LPAREN() (
#define G(Q) 42
#define F(R, X, ...) __VA_OPT__(G R X) )
int x = F(LPAREN(), 0, <:-);
#define H2(X, Y, ...) __VA_OPT__(X ## Y,) __VA_ARGS__
H2(a, b, c, d)
#define H3(X, ...) #__VA_OPT__(X##X X##X)
H3(, 0)
#define H4(X, ...) __VA_OPT__(a X ## X) ## b
H4(, 1)
#define H5A(...) __VA_OPT__()/**/__VA_OPT__()
#define H5B(X) a ## X ## b
#define H5C(X) H5B(X)
H5C(H5A())
"#;
    assert_eq!(pp_in(src, cpp()), toks(r#"int x = 42; ab, c, d "" a b ab"#));
}

#[test]
fn gnu_comma_and_named_variadics() {
    let src = "#define e(fmt, ...) p(fmt, ## __VA_ARGS__)\n#define n(fmt, args...) p(fmt, ## args)\ne(x) e(x, 1, 2) n(y) n(y, 3)\n";
    assert_eq!(pp(src), toks("p(x) p(x, 1, 2) p(y) p(y, 3)"));
}

#[test]
fn painted_names_stay_painted() {
    // `f(2)(9)` (C17 §6.10.3.4's example): `g` is painted inside its own expansion.
    let src = "#define f(a) a*g\n#define g(a) f(a)\nf(2)(9)\n#define AA BB\n#define BB AA\nAA BB\n";
    assert_eq!(pp(src), toks("2*9*g AA BB"));
}

#[test]
fn conditionals() {
    let src = r#"#define A 2
#if A == 2 && defined(A) && !defined B
yes1
#elif garbage here 1/0
no1
#else
no2
#endif
#if 0
#if this is skipped (
#else
#endif
#error not seen
'unclosed
#elif defined A
yes2
#endif
#ifdef A
yes3
#endif
#ifndef A
no3
#elifndef B
yes4
#endif
#define D defined(A)
#if D
yes5
#endif
#if -1 > 0u
yes6
#endif
#if 1 ? 2 : (1 / 0)
yes7
#endif
"#;
    let (got, facts) = run_with(&[("main.c", src)], &[], c());
    assert_eq!(got.join(" "), "yes1 yes2 yes3 yes4 yes5 yes6 yes7");
    assert!(facts.diags.is_empty(), "{:?}", facts.diags);
    let taken: Vec<bool> = facts.conds.iter().map(|c| c.2).collect();
    // `#elif` and `#else` after a taken group are not taken; a nested `#if` in a skipped group is
    // not evaluated.
    assert_eq!(taken, [true, false, false, false, true, true, false, true, true, true, true]);
}

#[test]
fn includes_once_guards_and_next() {
    let files = [
        ("main.c", "#include \"a.h\"\n#include \"a.h\"\n#include <b.h>\n#include \"g.h\"\n#include \"g.h\"\n#import \"i.h\"\n#import \"i.h\"\n#include <none.h>\nend\n"),
        ("a.h", "#pragma once\nA\n"),
        ("inc1/b.h", "B1\n#include_next <b.h>\n"),
        ("inc2/b.h", "B2\n"),
        ("g.h", "#ifndef G\n#define G\nG_body\n#endif\n"),
        ("i.h", "I\n"),
    ];
    let (got, facts) = run_with(&files, &[("inc1", DirKind::Angle), ("inc2", DirKind::System)], c());
    assert_eq!(got.join(" "), "A B1 B2 G_body I end");
    let to: Vec<&str> = facts.included.iter().map(|x| facts.paths[x.2 as usize].as_str()).collect();
    assert_eq!(to, ["a.h", "inc1/b.h", "inc2/b.h", "g.h", "g.h", "i.h"]);
    assert_eq!(facts.missing.iter().map(|m| (m.2.as_slice(), m.3)).collect::<Vec<_>>(), [(b"none.h".as_slice(), true)]);
    assert!(facts.diags.iter().any(|d| d.2 == Diag::NotFound));
}

#[test]
fn builtins_and_pragmas() {
    let files = [("main.c", "__LINE__ __FILE__ __COUNTER__ __COUNTER__\n#include \"h.h\"\n#line 100 \"x.c\"\n__LINE__ __FILE__\n#define L __LINE__\nL\n_Pragma(\"omp parallel\") z\n#pragma push_macro(\"Q\")\n#define Q 1\nQ\n#pragma pop_macro(\"Q\")\nQ\n#if __has_include(\"h.h\") && !__has_include(<nope.h>) && __has_feature(f)\nhas\n#endif\n"), ("h.h", "__INCLUDE_LEVEL__ __FILE_NAME__ __BASE_FILE__\n")];
    let mut cfg = c();
    cfg.features.insert(("__has_feature".into(), "f".into()), 1);
    let (got, facts) = run_with(&files, &[], cfg);
    assert_eq!(got.join(" "), r#"1 "main.c" 0 1 1 "h.h" "main.c" 100 "x.c" 102 z 1 Q has"#);
    assert_eq!(facts.pragmas.iter().map(|p| String::from_utf8_lossy(&p.2).into_owned()).collect::<Vec<_>>(), ["omp parallel", "push_macro(\"Q\")", "pop_macro(\"Q\")"]);
}

#[test]
fn predefines_and_expansion_facts() {
    let mut cfg = c();
    cfg.define("X=1").define("F(a)=a+X").define("Y").undef("Y");
    let (got, facts) = run_with(&[("main.c", "F(2) X Y\n")], &[], cfg);
    assert_eq!(got.join(" "), "2 + 1 1 Y");
    let names: Vec<(u32, &[u8], u32)> = facts.expansions.iter().map(|e| (e.0, e.1.as_slice(), e.6)).collect();
    // `X` inside `F`'s expansion is expansion 2's, from expansion 1.
    assert_eq!(names, [(1, b"F".as_slice(), 0), (2, b"X".as_slice(), 1), (3, b"X".as_slice(), 0)]);
}

#[test]
fn diagnostics() {
    let (_, facts) = run_with(&[("main.c", "#if 1\n#error stop \"here\"\n#warning w\n#define F(a) a\nF(1, 2)\nF(\n")], &[], c());
    let kinds: Vec<&Diag> = facts.diags.iter().map(|d| &d.2).collect();
    assert_eq!(kinds, [&Diag::Error, &Diag::Warning, &Diag::ArgCount, &Diag::UnterminatedCall, &Diag::Unterminated]);
    assert_eq!(facts.diags[0].3, b"stop \"here\"");
}

#[test]
fn text_sink_reads_back_as_its_tokens() {
    let src = "#define cat(a, b) a ## b\n#define S(x) #x\nint cat(x, 1) = 1+ +2; S( a  \"b\\n\" );\n\n\nx- -y .5 e+\n";
    let mut io = Search::new(Vec::new(), mem(&[("main.c", src)]));
    let (mut text, mut sp) = (crate::sink::TextSink::default(), Spellings::default());
    let mut tee = Tee(&mut text, &mut sp);
    let mut pp = Pp::new(c(), &mut io, &mut tee);
    pp.run("main.c");
    drop(pp);
    let out = String::from_utf8(text.out.clone()).unwrap();
    let body: String = out.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n");
    assert_eq!(toks(&body), sp.0.join(" "), "{out}");
    assert!(out.starts_with("# 3 \"main.c\"\n"), "{out}");
}
