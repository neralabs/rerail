use super::*;

/// The grammars by the interpreter (the same tapes as every backend's, `tokens::tests`), so a
/// test compiles no grammar through Cranelift.
const INTERPRETED: Grammars = Grammars { tokens: &tokens::interpreted, read: &crate::interpreted_read };

/// A TU: its name, and its files (each one's path and text, the main file first).
type Tu<'a> = (String, Vec<(&'a str, &'a [u8])>);

/// The words over a TU of one file, `main.c`.
fn expand(src: &[u8], cpp: bool) -> Result<Vec<Out>, String> {
    let files = [("main.c", src)];
    expand_tu("main.c", &mut search(&files, &[]), cpp, &INTERPRETED)
}

/// The words' tokens equal the lean preprocessor's, kinds, spellings and flags, in C and C++.
#[track_caller]
fn same(src: &str) {
    for cpp in [false, true] {
        let got = expand(src.as_bytes(), cpp).unwrap_or_else(|e| panic!("cpp {cpp}: {e}\n{src}"));
        let want = lean(src.as_bytes(), cpp);
        let show = |v: &[Out]| v.iter().map(|(k, f, s)| format!("{}:{k}:{f}", String::from_utf8_lossy(s))).collect::<Vec<_>>().join(" ");
        assert_eq!(show(&got), show(&want), "cpp {cpp}\n{src}");
    }
}

/// The examples that tell the dialects apart do: the lean pass expands each differently in C
/// and C++, so equal to it in both, the words read the run's dialect (`Env::cpp`).
#[test]
fn the_dialects_examples_differ() {
    for name in ["pastes by dialect", "#if, #elif and their expressions"] {
        let src = EXAMPLES.iter().find(|e| e.0 == name).unwrap().1;
        assert_ne!(lean(src.as_bytes(), false), lean(src.as_bytes(), true), "{name}");
    }
}

#[test]
fn the_program_assembles() {
    // One program; C++'s rows in it, applied where the run is C++'s.
    let p = program().unwrap();
    assert!(p.pp.suffix && p.pp.cpp && p.pp.cpp_ops.len() == 3);
}

/// The examples: the standard's (C17 §6.10.3.3–5, C++20's `__VA_OPT__`) and the cases the lean
/// pass's tests name. One test, so the token grammar is lowered once a process.
const EXAMPLES: &[(&str, &str)] = &[
    (
        "C17 example 3, rescanning",
        r#"#define x 3
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
"#,
    ),
    (
        "C17 example 4, strings and pastes",
        r#"#define str(s) # s
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
xstr(INCFILE(2).h)
glue(HIGH, LOW);
xglue(HIGH, LOW)
"#,
    ),
    ("C17 example 5, placemarkers", "#define t(x,y,z) x ## y ## z\nint j[] = { t(1,2,3), t(,4,5), t(6,,7), t(8,9,),\n t(10,,), t(,11,), t(,,12), t(,,) };\n"),
    (
        "C17 example 7, variadic",
        r#"#define debug(...) fprintf(stderr, __VA_ARGS__)
#define showlist(...) puts(#__VA_ARGS__)
#define report(test, ...) ((test)?puts(#test):\
 printf(__VA_ARGS__))
debug("Flag");
debug("X = %d\n", x);
showlist(The first, second, and third items.);
report(x>y, "x is %d but y is %d", x, y);
"#,
    ),
    (
        "__VA_OPT__",
        r#"#define F(...) f(0 __VA_OPT__(,) __VA_ARGS__)
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
"#,
    ),
    (
        "__VA_OPT__, C++20's examples",
        r#"#define LPAREN() (
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
"#,
    ),
    ("GNU's comma and named variadics", "#define e(fmt, ...) p(fmt, ## __VA_ARGS__)\n#define n(fmt, args...) p(fmt, ## args)\ne(x) e(x, 1, 2) n(y) n(y, 3)\n"),
    ("painted names stay painted", "#define f(a) a*g\n#define g(a) f(a)\nf(2)(9)\n#define AA BB\n#define BB AA\nAA BB\n"),
    ("calls across lines", "#define f(a, b) [a|b]\nf(1,\n2) f(1) f\n(3, 4) f + f(\n#define g 5\ng, g)\n"),
    ("wrong counts, no `)`", "#define f(a) a\nf(1, 2) f() f(f(f)) x f(\n"),
    ("no parameters", "#define e() E\ne() e( ) e(1) e\n"),
    (
        "pastes of every kind",
        concat!(
            "#define P(a, b) a ## b\n",
            "P(x, y) P(x, 12) P(x, 1.5) P(1, e) P(1e, +) P(1, .) P(1, +) P(., 5) P(., x)\n",
            "P(-, >) P(<, <=) P(<, :) P(%, >) P(#, #) P(/, /) P(+, -) P(L, \"s\") P(u8, 'c') P(x, \"s\")\n",
            "P(\"s\", _x) P(\"s\", 1) P(-, 1) P(, ) P(a, ) P(, b) P(., .) P(.., .)\n",
        ),
    ),
    ("stringizing", "#define s(x) #x\n#define t(...) #__VA_ARGS__\ns(  a  +  b ) s(\"q\\\"\" 'c' \\n) s() t(a,  b , c) s(\n  x\n  y)\n"),
    (
        "#ifdef, #ifndef, #else, #endif, nested",
        "#define A\n#ifdef A\na1\n#ifdef B\nb1\n#else\nb2\n#endif\n#else\na2\n#ifdef A\nx\n#endif\n#endif\n#ifndef B\nnb\n#endif\n#ifndef A\nna\n#else\nya\n#endif\n#ifdef\nm1\n#endif\n#ifndef\nm2\n#endif\n",
    ),
    (
        "#if, #elif and their expressions",
        concat!(
            "#define N 3\n#define F(x) (x * 2)\n",
            "#if N > 2 && F(N) == 6\nyes1\n#elif 1\nno1\n#else\nno2\n#endif\n",
            "#if 0\nno3\n#elif N - 3\nno4\n#elif defined(N) && !defined M\nyes2\n#elif 1\nno5\n#endif\n",
            "#if (1 ? 2 : 1/0) == 2 && -1 < 0u == 0 && 0x10 >> 2 == 4 && '\\n' == 10 && 1, 1\nyes3\n#endif\n",
            "#if\nno6\n#elif 1/0\nno7\n#else\nyes4\n#endif\n",
            "#if undefined_name || true\nmaybe\n#endif\n",
            "#if 1 and 2\nalt\n#endif\n",
            "#if 0\n#if 1\nno8\n#else\nno9\n#endif\n#elif 2\nyes5\n#endif\n",
            "#define D defined(N)\n#if D\nyes6\n#endif\n",
        ),
    ),
    (
        "#if's evaluator, its edges",
        concat!(
            "#if 0 && 1/0\nno1\n#elif 1 || 1%0\nyes1\n#endif\n",
            "#if 1 ? 1 : 1/0\nyes2\n#endif\n#if 0 ? 1/0 : 3\nyes3\n#endif\n#if 1 ? 0 : 1/0\nno2\n#endif\n",
            "#if (0 ? 1u : -1) > 0\nyes4\n#endif\n#if -1 > 0u\nyes5\n#endif\n#if (-1 >> 63) == -1 && (1u << 64) == 0\nyes6\n#endif\n",
            "#if 1 ? 2, 0 : 3\nno3\n#endif\n#if 0, 1\nyes7\n#endif\n#if 1 ? 0 ? 5 : 6 : 7\nyes8\n#endif\n",
            "#if ((((((((((((((((((((1))))))))))))))))))))\nyes9\n#endif\n#if - - ~ ! 0 == -2\nno4\n#elif !!7 == 1\nyes10\n#endif\n",
            "#if (1\nno5\n#endif\n#if 1)\nno6\n#endif\n#if 1 +\nno7\n#endif\n#if 1 ? 2\nno8\n#endif\n#if 1.5\nno9\n#endif\n#if \"s\"\nno10\n#endif\n",
            "#if 2 + 3 * 4 - 10 / 2 % 3 == 12 && (5 & 3 | 8 ^ 1) == 9 && 1 < 2 == 1 >= 0\nyes11\n#endif\n",
            "#if 1 1\nno11\n#endif\n#if ,1\nno12\n#endif\n#if 1 = 1\nno13\n#endif\n",
        ),
    ),
    ("#elifdef, #elifndef", "#define A\n#ifdef Z\n#elifdef A\nyes\n#elifndef A\nno\n#endif\n#ifdef Z\n#elifndef Z\nyes2\n#else\nno\n#endif\n"),
    ("unmatched", "a\n#endif\nb\n#else\nc\n#elif 1\nd\n"),
    (
        "__LINE__, __FILE__, #line and the other built-ins",
        concat!(
            "#define L __LINE__\n#define F(x) x __LINE__\na __LINE__ L\nF(\n1\n)\n",
            "__FILE__ __FILE_NAME__ __BASE_FILE__ __INCLUDE_LEVEL__ __COUNTER__ __COUNTER__\n",
            "#line 100\n__LINE__\n#line 200 \"dir/other.c\"\n__LINE__ __FILE__ __FILE_NAME__\n",
            "# 300 \"marker.c\" 1\n__LINE__ __FILE__\n#define N 400\n#line N\n__LINE__\n#line x\n__LINE__\n",
            "__DATE__ __TIME__ __TIMESTAMP__\n#if defined(__LINE__) && __LINE__ > 400\ny\n#endif\n",
            "#define S(x) #x\n#define X(x) S(x)\nX(__LINE__) S(__LINE__)\n",
            "#undef __FILE__\n__FILE__\n#define __COUNTER__ c\n__COUNTER__\n",
        ),
    ),
    (
        "__has_include and the feature tests",
        concat!(
            "#if __has_include(\"main.c\")\nyes\n#endif\n#if __has_include(<none.h>) || __has_include_next(\"none.h\")\nno\n#endif\n",
            "#if __has_feature(cxx_x) || __has_attribute(noreturn) || __has_cpp_attribute(nodiscard)\nno2\n#endif\n",
            "#if __is_identifier(foo) && defined(__has_include)\nid\n#endif\n#define H \"main.c\"\n#if __has_include(H)\nyes2\n#endif\n",
            "__has_builtin(x) __has_feature __has_include(\"main.c\") __has_embed(\"main.c\")\n",
        ),
    ),
    (
        "_Pragma",
        concat!(
            "#define M 1\n_Pragma(\"push_macro(\\\"M\\\")\")\n#undef M\nM\n_Pragma(\"pop_macro(\\\"M\\\")\")\nM\n",
            "#define P(x) _Pragma(#x) after\nP(message(\"hi\"))\n_Pragma\nx\n_Pragma(1) y\n#define Q \"once\"\n_Pragma(Q) z\n",
            // Its text lexed as a file is: a comment a blank, and the same text twice one file.
            "#define N 2\n_Pragma(\" push_macro/**/( \\\"N\\\" ) \")\n#undef N\nN\n_Pragma(\"pop_macro(\\\"N\\\")\")\nN\n_Pragma(\"pop_macro(\\\"N\\\")\") N\n",
        ),
    ),
    // C++'s punctuators and literal suffixes paste into one token in C++ only.
    ("pastes by dialect", "#define P(a, b) a ## b\nP(->, *) P(<=, >) P(., *) P(\"s\", _x) P(1, _x)\n"),
    ("undefines and redefinitions", "#define A 1\nA\n#undef A\nA\n#define A 2\nA\n#undef B\n#define F(x) x\n#undef F\nF(1)\n# \n#pragma x\nA\n"),
];

/// A group open at the file's end: mode (a) refuses the file, so the TU goes to the lean pass.
#[test]
fn an_unterminated_group_is_refused() {
    let e = expand(b"#define A\n#ifdef A\nopen\n#else\nnever\n", false).err().unwrap();
    assert!(e.contains("directives' grammar"), "{e}");
}

/// TUs of several files: each its files (the first the main file) and its search path (`inc`, `-I`;
/// `sys` and `sys2`, `-isystem`).
const TUS: &[(&str, &[(&str, &str)])] = &[
    (
        "quote and angle includes, a guard, not found",
        &[
            ("main.c", "#include \"a.h\"\n#include <b.h>\n#include \"a.h\"\nA B\n#include \"none.h\"\n#include <none.h>\nend\n"),
            ("a.h", "#ifndef A_H\n#define A_H\n#define A 1\nin_a\n#endif\n"),
            ("inc/b.h", "#define B 2\nin_b\n"),
        ],
    ),
    (
        "#pragma once, #import",
        &[
            ("main.c", "#include \"o.h\"\n#include \"o.h\"\n#import \"i.h\"\n#include \"i.h\"\n#import \"i.h\"\nend\n"),
            ("o.h", "#pragma once\nonce\n"),
            ("i.h", "imported\n"),
        ],
    ),
    (
        "names made by macros",
        &[
            ("main.c", "#define Q \"q.h\"\n#define A <b.h>\n#define F(x) #x\n#include Q\n#include A\n#include F(q.h)\n#include W\n#define N(x) <x.h>\n#include N(b)\nend\n"),
            ("q.h", "in_q\n"),
            ("inc/b.h", "in_b\n"),
        ],
    ),
    (
        "#include_next",
        &[
            ("main.c", "#include <n.h>\nend\n"),
            ("sys/n.h", "first\n#include_next <n.h>\n"),
            ("sys2/n.h", "second\n#include_next <n.h>\n"),
        ],
    ),
    (
        "macros and conditionals across files",
        &[
            ("main.c", "#define X 1\n#include \"c.h\"\nY Z\n#if defined(Y) && Y == 3\nyes\n#endif\n#include \"call.h\"\n2)\n"),
            ("c.h", "#if X\n#define Y 3\n#else\n#define Y 4\n#endif\n#ifdef X\n#include \"d.h\"\n#endif\n"),
            ("d.h", "#define Z X+Y\n"),
            ("call.h", "#define G(a, b) a b\nG(1,\n"),
        ],
    ),
    (
        "built-ins across files",
        &[
            ("main.c", "#include \"h.h\"\n__FILE__ __LINE__ __INCLUDE_LEVEL__\n#if __has_include(\"h.h\") && !__has_include(\"no.h\")\nyes\n#endif\n#include \"h.h\"\n#include \"l.h\"\n__LINE__ __FILE__\n"),
            ("h.h", "_Pragma(\"once\")\n__FILE__ __LINE__ __INCLUDE_LEVEL__ __BASE_FILE__\n#if __has_include_next(<h.h>)\nno\n#endif\n"),
            ("l.h", "#line 50 \"elsewhere.h\"\n__LINE__ __FILE__\n"),
        ],
    ),
    (
        "#embed and __has_embed",
        &[
            (
                "main.c",
                concat!(
                    "#embed \"d.bin\"\n#embed <d.bin> limit(2) prefix(p,) suffix(,s)\n#embed \"e.bin\" if_empty(empty) prefix(no)\n",
                    "#define L 1+2\n#embed \"d.bin\" limit(L) unknown(x) if_empty(not)\n#define NAME \"d.bin\"\n#embed NAME suffix(;)\n",
                    "#define P prefix(q,)\n#embed \"d.bin\" P\n#embed \"none.bin\" if_empty(x)\n#embed\n#embed \"d.bin\" limit(0) if_empty(zero)\n",
                    "#if __has_embed(\"d.bin\") == 1 && __has_embed(\"e.bin\") == 2 && !__has_embed(\"none\")\nyes\n#endif\n",
                    "#embed \"d.bin\" prefix((a)(b),) limit((1)+(2)) if_empty()\n#embed \"d.bin\" limit(1/0)\n#embed \"d.bin\" limit(2 3)\n",
                    "#embed \"d.bin\" limit(4) suffix(x (y)\n",
                    "#include \"d.bin\"\nend\n",
                ),
            ),
            ("d.bin", "AB\u{0}~\n"),
            ("e.bin", ""),
            ("inc/d.bin", "xy"),
        ],
    ),
    (
        "push_macro and pop_macro",
        &[(
            "main.c",
            "#define M 1\n#pragma push_macro(\"M\")\n#undef M\n#define M 2\nM\n#pragma pop_macro(\"M\")\nM\n#pragma push_macro(\"U\")\n#define U 3\nU\n#pragma pop_macro(\"U\")\nU\n#pragma pop_macro(\"V\")\n#pragma push_macro(R\"(M)\")\n",
        )],
    ),
];

/// A TU's tokens by the words and by the lean pass, in C and C++.
fn same_tu(files: &[(&str, &str)]) -> Result<(), String> {
    let files: Vec<(&str, &[u8])> = files.iter().map(|f| (f.0, f.1.as_bytes())).collect();
    let dirs = [("inc", rerail_c_pp::files::DirKind::Angle), ("sys", rerail_c_pp::files::DirKind::System), ("sys2", rerail_c_pp::files::DirKind::System)];
    let show = |v: &[Out]| v.iter().map(|(k, f, s)| format!("{}:{k}:{f}", String::from_utf8_lossy(s))).collect::<Vec<_>>().join(" ");
    for cpp in [false, true] {
        let got = expand_tu(files[0].0, &mut search(&files, &dirs), cpp, &INTERPRETED).map_err(|e| format!("cpp {cpp}: {e}"))?;
        let want = lean_tu(files[0].0, &mut search(&files, &dirs), cpp);
        if show(&got) != show(&want) {
            return Err(format!("cpp {cpp}:\n words {}\n lean  {}", show(&got), show(&want)));
        }
    }
    Ok(())
}

#[test]
fn the_examples_expand_as_the_lean_pass_does() {
    for (name, files) in TUS {
        if let Err(e) = same_tu(files) {
            eprintln!("{name}: {e}");
        }
    }
    let tus: Vec<&str> = TUS.iter().filter(|(_, f)| same_tu(f).is_err()).map(|t| t.0).collect();
    let failed: Vec<String> = EXAMPLES.iter().filter_map(|(name, src)| std::panic::catch_unwind(|| same(src)).err().map(|_| (*name).to_owned())).collect();
    assert!(failed.is_empty() && tus.is_empty(), "differ: {failed:?} {tus:?}");
}

/// The built-ins' values, spelled out (the examples compare them with the lean pass's).
#[test]
fn the_builtins_have_their_values() {
    let got = expand(b"#define L __LINE__\n__LINE__\n\nL __FILE__\n#line 10 \"x.c\"\n__LINE__ __FILE__ __COUNTER__ __COUNTER__\n", false).unwrap();
    let words: Vec<String> = got.iter().map(|t| String::from_utf8_lossy(&t.2).into_owned()).collect();
    assert_eq!(words, ["2", "4", "\"main.c\"", "10", "\"x.c\"", "0", "1"]);
}

/// The words' built-ins are the lean pass's, in its order.
#[test]
fn the_builtins_are_the_lean_passs() {
    let words: Vec<(&str, bool)> = pp_words::BUILTINS.to_vec();
    let lean: Vec<(&str, bool)> = rerail_c_pp::macros::BUILTINS.iter().map(|b| (b.0, b.2)).collect();
    assert_eq!(words, lean);
}

/// The corpus's files, each a TU of its own: the words' tokens equal the lean pass's on every
/// file the grammars read.
#[test]
#[ignore = "reads the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS)"]
fn the_corpus_expands_as_the_lean_pass_does() {
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let (mut same, mut out, mut differ) = (0, std::collections::BTreeMap::<String, usize>::new(), Vec::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            let src = std::fs::read(&p).unwrap();
            if crate::interpreted_read(&src).is_none() {
                *out.entry("the directives' grammar refuses it".into()).or_default() += 1;
                continue;
            }
            if tokens::interpreted(&src, cpp).is_none() {
                *out.entry("the token grammar refuses it".into()).or_default() += 1;
                continue;
            }
            match expand(&src, cpp) {
                Err(e) => *out.entry(e.chars().take(80).collect()).or_default() += 1,
                Ok(got) if got == lean(&src, cpp) => same += 1,
                Ok(got) => {
                    let want = lean(&src, cpp);
                    let k = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
                    let show = |v: &[Out]| v[k.saturating_sub(2)..(k + 3).min(v.len())].iter().map(|(k, f, s)| format!("{}:{k}:{f}", String::from_utf8_lossy(s))).collect::<Vec<_>>().join(" ");
                    differ.push(format!("{}: token {k} of {}/{}: words [{}] lean [{}]", p.display(), got.len(), want.len(), show(&got), show(&want)));
                }
            }
        }
    }
    eprintln!("the same {same}, not run {}, differ {}", out.values().sum::<usize>(), differ.len());
    for (why, n) in &out {
        eprintln!("  {n:5} {why}");
    }
    for d in differ.iter().take(15) {
        eprintln!("  {d}");
    }
    assert!(differ.is_empty());
}

/// The compiled words (`native_cpu_pp`, decision 0348's stage 4b) give the interpreter's tokens
/// on every example and TU, in C and C++.
#[test]
fn the_compiled_words_are_the_interpreters() {
    let dirs = [("inc", rerail_c_pp::files::DirKind::Angle), ("sys", rerail_c_pp::files::DirKind::System), ("sys2", rerail_c_pp::files::DirKind::System)];
    let mut tus: Vec<Tu<'_>> = EXAMPLES.iter().map(|(n, s)| ((*n).to_owned(), vec![("main.c", s.as_bytes())])).collect();
    tus.extend(TUS.iter().map(|(n, f)| ((*n).to_owned(), f.iter().map(|f| (f.0, f.1.as_bytes())).collect())));
    let (mut same, mut differ) = (0, Vec::new());
    {
        let c = rerail_cpu_pp::native_cpu_pp::compile(program().unwrap()).unwrap();
        let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| c.run(w, f, e, b);
        // One compiled program; the dialect is the run's.
        for (cpp, (name, files)) in [false, true].into_iter().flat_map(|cpp| tus.iter().map(move |t| (cpp, t))) {
            let want = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &pp_words::run);
            let got = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &jit);
            match got {
                got if got == want => same += 1,
                got => differ.push(format!("{name} (cpp {cpp}): JIT {got:?}\n interpreter {want:?}")),
            }
        }
    }
    assert!(differ.is_empty(), "{} of {} differ:\n{}", differ.len(), same + differ.len(), differ.join("\n"));
}

/// The words on the process's device (`clif_ukl`'s UKL of the same IR, decisions 0348 stage 5, 0440 step D) are the
/// interpreter's, on every example and TU, in C and C++: on Metal, OpenCL and the CPU's backend (`RERAIL_GPU`), all of
/// them one batch a round (`expand_tus_with`, 0408 L3), and the host evaluates nothing meanwhile (`host_runs`).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn the_device_words_are_the_interpreters() {
    if rerail_metal::lane::device().is_err() {
        return;
    }
    let dirs = [("inc", rerail_c_pp::files::DirKind::Angle), ("sys", rerail_c_pp::files::DirKind::System), ("sys2", rerail_c_pp::files::DirKind::System)];
    let mut tus: Vec<Tu<'_>> = EXAMPLES.iter().map(|(n, s)| ((*n).to_owned(), vec![("main.c", s.as_bytes())])).collect();
    tus.extend(TUS.iter().map(|(n, f)| ((*n).to_owned(), f.iter().map(|f| (f.0, f.1.as_bytes())).collect())));
    let all: Vec<(bool, &Tu<'_>)> = [false, true].into_iter().flat_map(|cpp| tus.iter().map(move |t| (cpp, t))).collect();
    let mut io: Vec<_> = all.iter().map(|(_, (_, files))| search(files, &dirs)).collect();
    let mut batch: Vec<(&str, &mut _, bool)> = all.iter().zip(io.iter_mut()).map(|((cpp, (_, files)), io)| (files[0].0, io, *cpp)).collect();
    let runs = pp_words::host_runs();
    let metal = crate::expand::expand_tus_with(&mut batch, &INTERPRETED, &crate::expand::on_device_batch);
    assert_eq!(pp_words::host_runs(), runs, "the host ran the words during the device's batches");
    let (mut same, mut differ) = (0, Vec::new());
    for ((cpp, (name, files)), got) in all.iter().zip(metal) {
        let cpp = *cpp;
        let want = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &pp_words::run);
        match got {
            got if got == want => same += 1,
            got => {
                // The first token that differs, and the tokens around it.
                let show = |r: &Result<Vec<Out>, String>, at: usize| match r {
                    Ok(v) => format!("{} tokens, from {at}: {:?}", v.len(), v.iter().skip(at.saturating_sub(12)).take(20).map(|t| (t.0, t.1, String::from_utf8_lossy(&t.2).into_owned())).collect::<Vec<_>>()),
                    Err(e) => e.clone(),
                };
                let at = match (&got, &want) {
                    (Ok(g), Ok(w)) => g.iter().zip(w).position(|(a, b)| a != b).unwrap_or(g.len().min(w.len())),
                    _ => 0,
                };
                differ.push(format!("{name} (cpp {cpp}): device {}\n interpreter {}", show(&got, at), show(&want, at)));
            }
        }
    }
    assert!(differ.is_empty(), "{} of {} differ:\n{}", differ.len(), same + differ.len(), differ.join("\n"));
}

/// The compiled words translate to Rust (`clif_rust`, the `generated` feature's source), each
/// function's nesting shallow enough for rustc. `RERAIL_PP_AOT_DIR` set: written there.
#[test]
fn the_words_translate_to_rust() {
    {
        let src = rerail_cpu_pp::native_cpu_pp::rust_source(program().unwrap()).unwrap();
        let (mut depth, mut deepest) = (0usize, 0usize);
        for c in src.bytes() {
            match c {
                b'{' => {
                    depth += 1;
                    deepest = deepest.max(depth);
                }
                b'}' => depth -= 1,
                _ => {}
            }
        }
        eprintln!("{} KB, {} lines, nesting {deepest}", src.len() >> 10, src.lines().count());
        if let Ok(dir) = std::env::var("RERAIL_PP_AOT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join("pp.rs"), &src).unwrap();
        }
    }
}

/// The run's function as Cranelift compiles it: its instructions, its accesses
/// relative to the stack pointer (the register allocator's spills and reloads, and the frame's),
/// its branches by kind (before emission: a jump to the next block still there); its text and
/// its emitted bytes written to `RERAIL_PP_DISASM` (and `.bin`) where set (Q184: beside rustc's code of the
/// same IR, `the_aot_words_are_the_interpreters`' builds).
#[test]
#[ignore = "a report (Q184)"]
fn the_compiled_words_code_counted() {
    let c = rerail_cpu_pp::native_cpu_pp::disassemble(program().unwrap()).unwrap();
    let text = &c.disasm[0];
    let insts: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.ends_with(':') && !l.starts_with(';')).collect();
    let count = |f: &dyn Fn(&str) -> bool| insts.iter().filter(|l| f(l)).count();
    let sp = count(&|l| l.contains("[sp"));
    let (ldr_sp, str_sp) = (count(&|l| l.contains("[sp") && l.starts_with("ld")), count(&|l| l.contains("[sp") && l.starts_with("st")));
    let b = count(&|l| l.starts_with("b ") || l.starts_with("b\t"));
    let bcond = count(&|l| l.starts_with("b.") || l.starts_with("cbz") || l.starts_with("cbnz") || l.starts_with("tbz") || l.starts_with("tbnz"));
    let br = count(&|l| l.starts_with("br "));
    let mov = count(&|l| l.starts_with("mov "));
    eprintln!("Cranelift, C, k 0, the run's function: {} instructions, {sp} at sp ({ldr_sp} loads, {str_sp} stores), {b} jumps, {bcond} conditional branches, {br} indirect, {mov} moves", insts.len());
    if let Ok(path) = std::env::var("RERAIL_PP_DISASM") {
        std::fs::write(&path, text).unwrap();
        // Its bytes as emitted (jumps to the next block dropped), to disassemble beside rustc's.
        std::fs::write(format!("{path}.bin"), &c.code[0]).unwrap();
    }
}

/// The words' code built ahead of time (`clif_rust`, the `generated` feature) gives the
/// interpreter's tokens on every example and TU, for each build; each build is the source the
/// words translate to now (its fingerprint), so it is the JIT's code compiled by rustc.
#[cfg(all(feature = "generated", target_arch = "aarch64"))]
#[test]
fn the_aot_words_are_the_interpreters() {
    let dirs = [("inc", rerail_c_pp::files::DirKind::Angle), ("sys", rerail_c_pp::files::DirKind::System), ("sys2", rerail_c_pp::files::DirKind::System)];
    let mut tus: Vec<Tu<'_>> = EXAMPLES.iter().map(|(n, s)| ((*n).to_owned(), vec![("main.c", s.as_bytes())])).collect();
    tus.extend(TUS.iter().map(|(n, f)| ((*n).to_owned(), f.iter().map(|f| (f.0, f.1.as_bytes())).collect())));
    let (mut same, mut differ) = (0, Vec::new());
    if let Some(rerail_pp_aot::Build { run: entry, source, structured: (structured, functions), .. }) = rerail_pp_aot::BUILD {
        // The CPU path takes it: made from these words.
        assert!(aot_build().is_some(), "the build is not of the words' text");
        // Each function structured control, as rustc compiles it best (not a loop over a `match`).
        assert_eq!(structured, functions);
        let now = rerail_cpu_pp::native_cpu_pp::rust_source(program().unwrap()).unwrap();
        let fp = now.lines().find_map(|l| l.strip_prefix("pub const SOURCE: u64 = ")).unwrap();
        assert_eq!(fp.trim_end_matches(';'), format!("{source:#x}"), "the build is not the words' code now; rebuild with the feature");
        // SAFETY: `entry` is `rust_source`'s `pp_run` for this program (the fingerprint).
        let aot = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| unsafe { rerail_cpu_pp::native_cpu_pp::run_aot(entry, w, f, e, b) };
        for (cpp, (name, files)) in [false, true].into_iter().flat_map(|cpp| tus.iter().map(move |t| (cpp, t))) {
            let want = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &pp_words::run);
            let got = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &aot);
            match got {
                got if got == want => same += 1,
                got => differ.push(format!("{name} (cpp {cpp}): AOT {got:?}\n interpreter {want:?}")),
            }
        }
    }
    assert!(differ.is_empty(), "{} of {} differ:\n{}", differ.len(), same + differ.len(), differ.join("\n"));
    eprintln!("{same} runs equal");
}

/// The corpus's files, each a TU of its own: the compiled words' tokens equal the interpreter's
/// on every file the grammars read, and the TUs one refuses the other refuses for the same reason.
#[test]
#[ignore = "reads the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS)"]
fn the_corpus_compiled_is_the_interpreters() {
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let (mut same, mut out, mut differ) = (0, std::collections::BTreeMap::<String, usize>::new(), Vec::new());
    let (mut t_words, mut t_jit) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        let c = rerail_cpu_pp::native_cpu_pp::compile(program().unwrap()).unwrap();
        let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| c.run(w, f, e, b);
        for p in paths {
            let src = std::fs::read(&p).unwrap();
            let files = [("main.c", src.as_slice())];
            let t0 = std::time::Instant::now();
            let want = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &pp_words::run);
            let t1 = std::time::Instant::now();
            let got = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &jit);
            let t2 = std::time::Instant::now();
            match (&got, &want) {
                (Err(a), Err(b)) if a == b => *out.entry(b.chars().take(80).collect()).or_default() += 1,
                _ if got == want => {
                    same += 1;
                    (t_words, t_jit) = (t_words + (t1 - t0), t_jit + (t2 - t1));
                }
                _ => differ.push(format!("{}: JIT {:?}\n interpreter {:?}", p.display(), got.as_ref().map(|v| v.len()), want.as_ref().map(|v| v.len()))),
            }
        }
    }
    eprintln!("the same {same}, refused by both {}, differ {}; the expanded TUs: interpreter {t_words:?}, JIT {t_jit:?} (the grammars' reads included)", out.values().sum::<usize>(), differ.len());
    for (why, n) in &out {
        eprintln!("  {n:5} {why}");
    }
    for d in differ.iter().take(15) {
        eprintln!("  {d}");
    }
    assert!(differ.is_empty());
}

/// The compiled words' shape (decision 0348, stage 4d): no call but of their own functions (the
/// large words', `OUTLINE`), no indirect or tail call, no speculation barrier, no function over 128 KB, and the returns
/// their stated dispatches (`Step::Return`).
#[test]
fn the_compiled_words_have_their_shape() {
    use rerail_compiler::native_cpu_shape::Step;
    let p = program().unwrap();
    let c = rerail_cpu_pp::native_cpu_pp::compile(p).unwrap();
    eprintln!("code {:?} bytes, {} CLIF instructions", c.code_bytes, c.clif.lines().filter(|l| l.starts_with("    ") && !l.trim_end().ends_with(':')).count());
    let mut by: Vec<(usize, usize)> = c.word_insts.iter().copied().enumerate().map(|(pc, n)| (n, pc)).collect();
    by.sort_unstable_by(|a, b| b.cmp(a));
    let named = |pc: usize| {
        let (op, mode, _) = rerail_compiler::program::split(p.words[pc]);
        format!("{pc} {op:#04x} m={mode}")
    };
    let top: Vec<String> = by.iter().take(25).map(|&(n, pc)| format!("{}: {n}", named(pc))).collect();
    eprintln!("  the words with the most: {}", top.join(", "));
    let mut per_op = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for (pc, &n) in c.word_insts.iter().enumerate() {
        let (op, mode, _) = rerail_compiler::program::split(p.words[pc]);
        let e = per_op.entry(format!("{op:#04x} m={mode}")).or_default();
        (e.0, e.1) = (e.0 + n, e.1 + 1);
    }
    let mut ops: Vec<_> = per_op.into_iter().collect();
    ops.sort_by_key(|o| std::cmp::Reverse(o.1.0));
    eprintln!("  by word: {}", ops.iter().take(25).map(|(k, (n, c))| format!("{k} {n}/{c}")).collect::<Vec<_>>().join(", "));
    let functions = c.clif.matches("function ").count();
    assert!(!c.clif.contains("call_indirect") && !c.clif.contains("return_call"));
    // No speculation barrier in Cranelift's code (`csdb`, which its Arm jump tables carry):
    // its tables are trees of compares (`native_cpu_codegen::switch_trees`).
    let kept = rerail_cpu_pp::native_cpu_pp::disassemble(p).unwrap();
    let csdb = kept.code.iter().flat_map(|f| f.as_chunks::<4>().0).filter(|w| u32::from_le_bytes(**w) == 0xd503_229f).count();
    assert!(kept.code.len() == c.code_bytes.len() && kept.code.iter().all(|f| !f.is_empty()));
    if cfg!(target_arch = "aarch64") {
        assert_eq!(csdb, 0, "a speculation barrier in Cranelift's code");
    }
    let calls = c.clif.lines().filter(|l| l.contains(" = call ") || l.trim_start().starts_with("call ")).count();
    assert_eq!(functions, c.code_bytes.len());
    assert!(calls > 0);
    assert!(c.code_bytes.iter().all(|&b| b <= 128 << 10), "{:?}", c.code_bytes);
    // One dispatch for each `RETURN`.
    let returns = p.words.iter().filter(|&&w| rerail_compiler::program::split(w).0 == rerail_compiler::program::RETURN).count();
    assert_eq!(c.heads.iter().filter(|&&h| h == Step::Return).count(), returns);
}

/// The corpus's TUs expanded by the words' interpreter, by the compiled words as Cranelift
/// compiles them and as rustc does (the `generated` feature: `rerail-pp-aot`, the same IR), and
/// by the lean pass: each's instructions retired, cycles and wall time on this thread, the runs
/// warmed first. The words' runs are their runs alone (the grammars' reads are tier 1's and mode
/// (a)'s); the lean pass's lexes too. A benchmark: through `scripts/dev/quiet-run`, at
/// `--profile profiling`.
#[test]
#[ignore = "a benchmark of the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS)"]
fn the_corpus_timed() {
    use rerail_compiler::native_cpu_codegen::retired;
    use std::cell::Cell;
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    #[derive(Default, Clone, Copy)]
    struct Cost {
        instructions: u64,
        cycles: u64,
        wall: std::time::Duration,
    }
    let timed = |cost: &Cell<Cost>, f: &mut dyn FnMut()| {
        let (i0, c0, t0) = (retired().0, retired().1, std::time::Instant::now());
        f();
        let (t1, (i1, c1)) = (std::time::Instant::now(), retired());
        let c = cost.get();
        cost.set(Cost { instructions: c.instructions + (i1 - i0), cycles: c.cycles + (c1 - c0), wall: c.wall + (t1 - t0) });
    };
    let (words, jit, lean_pass) = (Cell::new(Cost::default()), Cell::new(Cost::default()), Cell::new(Cost::default()));
    let aot = Cell::new(Cost::default());
    // Each timed run: its cycles, the interpreter's and Cranelift's, what it did (tokens in, `#`
    // at a line's start, tokens out, expansions), and rustc's cycles.
    let runs: std::cell::RefCell<Vec<[u64; 7]>> = Default::default();
    let last = Cell::new([0u64; 7]);
    let mut tus = 0;
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        let srcs: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let c = rerail_cpu_pp::native_cpu_pp::compile(program().unwrap()).unwrap();
        for pass in 0..2 {
            let on = Cell::new(pass == 1);
            let run_words = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
                let mut r = None;
                if on.get() {
                    let before = words.get().cycles;
                    timed(&words, &mut || r = Some(pp_words::run(w, f, e, b)));
                    let x = r.as_ref().unwrap().as_ref().ok();
                    let toks: usize = f.iter().map(|f| f.toks.len()).sum();
                    let hashes = f.iter().flat_map(|f| f.toks.iter()).filter(|t| t.kind == pp_words::HASH && t.flags & pp_words::BOL != 0).count();
                    let mut l = last.get();
                    l[0] += words.get().cycles - before;
                    l[2..6].copy_from_slice(&[toks as u64, hashes as u64, x.map_or(0, |x| x.toks.len() as u64), x.map_or(0, |x| x.exps.len() as u64)]);
                    last.set(l);
                } else {
                    r = Some(pp_words::run(w, f, e, b))
                }
                r.unwrap()
            };
            let run_jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
                let mut r = None;
                if on.get() {
                    let before = jit.get().cycles;
                    timed(&jit, &mut || r = Some(c.run(w, f, e, b)));
                    let mut x = last.get();
                    x[1] += jit.get().cycles - before;
                    last.set(x);
                } else {
                    r = Some(c.run(w, f, e, b))
                }
                r.unwrap()
            };
            for src in &srcs {
                let files = [("main.c", src.as_slice())];
                last.set([0; 7]);
                let want = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &run_words);
                if want.is_err() {
                    continue;
                }
                let got = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &run_jit);
                assert_eq!(got, want);
                // By rustc (where built).
                let at0 = |cost: &Cell<Cost>, slot: usize, run: &crate::expand::Runner<'_>| {
                    let g = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &|w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
                        let mut r = None;
                        let before = cost.get().cycles;
                        if on.get() { timed(cost, &mut || r = Some(run(w, f, e, b))) } else { r = Some(run(w, f, e, b)) }
                        let mut x = last.get();
                        x[slot] += cost.get().cycles - before;
                        last.set(x);
                        r.unwrap()
                    });
                    assert_eq!(g, want);
                };
                if let Some(rerail_pp_aot::Build { run: entry, .. }) = rerail_pp_aot::BUILD {
                    // SAFETY: `entry` is `rust_source`'s `pp_run` for the words (`BUILD`).
                    at0(&aot, 6, &|w, f, e, b| unsafe { rerail_cpu_pp::native_cpu_pp::run_aot(entry, w, f, e, b) });
                }
                if pass == 1 {
                    runs.borrow_mut().push(last.get());
                }
                if pass == 1 {
                    timed(&lean_pass, &mut || {
                        std::hint::black_box(lean(src, cpp));
                    });
                    tus += 1;
                }
            }
        }
    }
    let mut rows = vec![("the words' interpreter", words.get()), ("Cranelift", jit.get())];
    if aot.get().cycles > 0 {
        rows.push(("rustc (same IR)", aot.get()));
    }
    rows.push(("the lean pass (lexing too)", lean_pass.get()));
    for (name, c) in rows {
        eprintln!("{name:28} {:>8.1} M instructions {:>8.1} M cycles {:>10.3?}", c.instructions as f64 / 1e6, c.cycles as f64 / 1e6, c.wall);
    }
    eprintln!("over {tus} TUs");
    // The compiled words' cycles over the interpreter's, by what the runs do: a bucket's sums.
    let runs = runs.borrow();
    let ratio = |rs: &[&[u64; 7]], i: usize| rs.iter().map(|r| r[i]).sum::<u64>() as f64 / rs.iter().map(|r| r[0]).sum::<u64>().max(1) as f64;
    let share = |rs: &[&[u64; 7]]| rs.iter().map(|r| r[0]).sum::<u64>() as f64 / runs.iter().map(|r| r[0]).sum::<u64>().max(1) as f64;
    for (name, key) in [("tokens out a token in", 0usize), ("expansions a token in", 1), ("directives a token in", 2), ("tokens in", 3)] {
        let v = |r: &[u64; 7]| -> f64 {
            let toks = r[2].max(1) as f64;
            match key {
                0 => r[4] as f64 / toks,
                1 => r[5] as f64 / toks,
                2 => r[3] as f64 / toks,
                _ => r[2] as f64,
            }
        };
        let mut sorted: Vec<&[u64; 7]> = runs.iter().collect();
        sorted.sort_by(|a, b| v(a).total_cmp(&v(b)));
        let q = sorted.len() / 4;
        let parts: Vec<String> = (0..4).map(|i| {
            let part = &sorted[i * q..if i == 3 { sorted.len() } else { (i + 1) * q }];
            format!("{:.3}..{:.3}: Cranelift x{:.2}, rustc x{:.2} ({:.0}% of the interpreter's)", v(part[0]), v(part[part.len() - 1]), ratio(part, 1), ratio(part, 6), 100.0 * share(part))
        }).collect();
        eprintln!("by {name}, quarters: {}", parts.join("; "));
    }
}

/// `#if`'s primaries and operators, the JIT's against the interpreter's, value by value: each
/// case's 64 bits, whether it is unsigned, and whether it is an error, each an `#if` of its own.
#[test]
fn the_compiled_ifs_have_the_interpreters_values() {
    const CASES: &[&str] = &[
        // Numbers.
        "0", "7", "010", "0x1F", "0X1f", "0b101", "0B11", "1'000'000", "0'7", "0x'1", "18446744073709551615", "18446744073709551616", "9223372036854775808",
        "1u", "1U", "1ul", "1LU", "1ull", "1LLU", "1z", "1uz", "1ZU", "1wb", "1uwb", "1WBU", "1lul", "1uu", "08", "0x", "0b2", "1.5", "1e3", "077777777777777777777",
        "0777777777777777777777", "0xffffffffffffffff", "0x10000000000000000", "0b1111111111111111111111111111111111111111111111111111111111111111", "123456789012345678",
        // Character constants.
        "'a'", "'ab'", "'abcde'", "'\\n'", "'\\e'", "'\\x41'", "'\\x'", "'\\101'", "'\\1234'", "'\\0'", "'\\''", "'\\\\'", "'\\q'", "u8'a'", "u'a'", "U'a'", "L'a'",
        "L'\\xffffffff'", "u'\\xffff1'", "U'\\U0001F600'", "u'\\u00e9'", "u'\\u+0e9'", "u'\\u00'", "L'é'", "'é'", "U'😀'", "u8'é'", "L'\\xe9'", "'\\x80'", "'\\377'", "'ab\\n'",
        // Identifiers, operators.
        "true", "foo", "(1 and 1)", "-1 < 0u", "0 && 1/0", "1 || 1/0", "1 ? 2 : 1/0", "0 ? 1/0 : 3", "1/0", "1%0", "(-9223372036854775807-1) / -1", "(-9223372036854775807-1) % -1",
        "1 << 64", "1 << 63", "-1 >> 70", "-8 >> 1", "18446744073709551615u >> 63", "~0u", "!5", "-(0u)", "-1u", "7 / -2", "-7 % 2", "7u / 2", "3 * -4", "2 + 3 * 4 - 1",
        "1 == 1u", "2 != 3", "5 & 3 | 8 ^ 1", "-1 <= 0", "0u >= 1", "(1)", "((2))", "1 +", "(1", "1 2", "+-~!0", "1 ? 2 : 3 ? 4 : 5", "0 ? 2u : -1",
        // Deep: past every depth compiled (16 calls), and errors met deep.
        "((((((((((1))))))))))", "-(-(-(-(-(-(-(-(-(-1)))))))))", "!!!!!!!!!!!!!!!!!!0", "1 ? (2 ? (3 ? (4 ? 5 : 6) : 7) : 8) : 9", "1 || 2 && 3 | 4 ^ 5 & 6 == 7 < 8 << 9 + 10 * 11",
        "((((((((((1/0))))))))))", "(1, (2, (3, (4, 5))))", "((((((((((1)))))))))", "1 * (2 + (3 * (4 + (5 * (6 + (7 * 8))))))",
    ];
    let words = |v: &[Out]| v.iter().map(|t| String::from_utf8_lossy(&t.2).into_owned()).collect::<String>();
    let mut unlexed = Vec::new();
    let c = rerail_cpu_pp::native_cpu_pp::compile(program().unwrap()).unwrap();
    for cpp in [false, true] {
        let mut differ = Vec::new();
        for case in CASES {
            let mut src = format!("#if ({case}) == ({case})\nok\n#else\nerr\n#endif\n#if ({case}) * 0 - 1 > 0\nu\n#else\ns\n#endif\n");
            for bit in (0..64).rev() {
                src.push_str(&format!("#if (({case}) >> {bit}) & 1\n1\n#else\n0\n#endif\n"));
            }
            let files = [("main.c", src.as_bytes())];
            let want = match expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &pp_words::run) {
                Ok(w) => w,
                Err(e) if e.contains("token grammar refuses") => {
                    unlexed.push(format!("{case} (cpp {cpp})"));
                    continue;
                }
                Err(e) => panic!("{case} (cpp {cpp}): {e}"),
            };
            let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| c.run(w, f, e, b);
            let got = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &jit).unwrap_or_else(|e| panic!("{case} (cpp {cpp}): {e}"));
            if got != want {
                differ.push(format!("{case}: interpreter {} JIT {}", words(&want), words(&got)));
            }
        }
        assert!(differ.is_empty(), "cpp {cpp}:\n{}", differ.join("\n"));
    }
    eprintln!("the token grammar refuses {unlexed:?}");
}

/// The device's rungs (0348, stage L1): a TU whose expansion outgrows the smallest rung's rooms
/// (a macro of 8 macros of 8 macros: 4,096 tokens from 60) is run again at larger rungs, and
/// its tokens are the interpreter's.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn the_device_rungs_run_a_tu_again_past_its_room() {
    if rerail_metal::lane::device().is_err() {
        return;
    }
    let src = "#define A x x x x x x x x\n#define B A A A A A A A A\n#define C B B B B B B B B\nC C C C C C C C\n";
    let files = [("main.c", src.as_bytes())];
    let rungs = std::cell::Cell::new([0usize; pp_words::RUNGS]);
    let source = crate::expand::ukl().unwrap();
    let device = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
        let ran = rerail_metal::pp_on::run_batch(source, &[(w, f, e, b)], 1).expect("the batch");
        rungs.set(ran.rungs);
        ran.results.into_iter().next().expect("one result")
    };
    let want = expand_tu_with("main.c", &mut search(&files, &[]), false, &INTERPRETED, &pp_words::run).unwrap();
    let got = expand_tu_with("main.c", &mut search(&files, &[]), false, &INTERPRETED, &device).unwrap();
    assert_eq!(want.len(), 4096);
    assert!(got == want, "the device's tokens differ");
    assert_eq!(rungs.get()[0], 0, "the smallest rung held it: {:?}", rungs.get());
}

/// Stage 5's spike (0348): each round of `TUS` run on the device as batches of 1 to 128 copies
/// of itself, against the Cranelift JIT on one core. It prints; it asserts nothing.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
#[ignore = "a measurement: run alone, on an idle Mac"]
fn the_device_words_time_against_the_jit() {
    let t0 = std::time::Instant::now();
    let src = crate::expand::ukl().expect("ukl");
    let translate = t0.elapsed();
    let dirs = [("inc", rerail_c_pp::files::DirKind::Angle), ("sys", rerail_c_pp::files::DirKind::System), ("sys2", rerail_c_pp::files::DirKind::System)];
    let ns = [1usize, 32, 64, 128];
    let jit = rerail_cpu_pp::native_cpu_pp::compile(program().unwrap()).unwrap();
    let cpu = std::cell::Cell::new(0f64);
    let gpu = std::cell::RefCell::new([0f64; 4]);
    let calls = std::cell::Cell::new(0usize);
    let skipped = std::cell::RefCell::new([0usize; 4]);
    let bytes = std::cell::Cell::new(0usize);
    let first = std::cell::Cell::new(None);
    let run = |w: &Words, fl: &[File], e: &pp_words::Env, b: &Bounds| {
        let mut out = None;
        for (k, &n) in ns.iter().enumerate() {
            let jobs = vec![(w, fl, e, b); n];
            let t = std::time::Instant::now();
            let ran = match rerail_metal::pp_on::run_batch(src, &jobs, n) {
                Ok(r) => r,
                Err(err) => {
                    eprintln!("SPIKE n={n} refused ({err:?}) after {} bytes a run", bytes.get());
                    skipped.borrow_mut()[k] += 1;
                    continue;
                }
            };
            if first.get().is_none() {
                first.set(Some(t.elapsed()));
            }
            gpu.borrow_mut()[k] += ran.gpu_seconds;
            if n == 1 {
                bytes.set(bytes.get().max(ran.arena_bytes));
                out = Some(ran.results.into_iter().next().unwrap());
            }
        }
        calls.set(calls.get() + 1);
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let _ = jit.run(w, fl, e, b);
            best = best.min(t.elapsed().as_secs_f64());
        }
        cpu.set(cpu.get() + best);
        out.unwrap()
    };
    for (_, files) in TUS {
        let files: Vec<(&str, &[u8])> = files.iter().map(|f| (f.0, f.1.as_bytes())).collect();
        let _ = expand_tu_with(files[0].0, &mut search(&files, &dirs), false, &INTERPRETED, &run);
    }
    let g = gpu.borrow();
    eprintln!("SPIKE CPU (Cranelift JIT, one core, least of 5, prepare and complete included): {:.3} ms total, {:.1} us a run", cpu.get() * 1e3, cpu.get() * 1e6 / calls.get() as f64);
    eprintln!("SPIKE ukl {} lines, translate {:?}, first batch (pipeline compile) {:?}", src.lines().count(), translate, first.get().unwrap());
    eprintln!("SPIKE {} runs ({} TUs), largest arena {} bytes", calls.get(), TUS.len(), bytes.get());
    for (k, &n) in ns.iter().enumerate() {
        let done = calls.get() - skipped.borrow()[k];
        eprintln!("SPIKE n={n}: {done} batches, GPU {:.3} ms total, {:.1} us a run", g[k] * 1e3, g[k] * 1e6 / (done * n).max(1) as f64);
    }
}

/// The device's rungs (0348, stage L1) on the corpus: each round run by the JIT from the smallest
/// rung up (`Bounds::rung`), as the device runs it, against the run at `Bounds::of`. It prints
/// the rung each round needed, the reasons a smaller one refused, and the device bytes a run
/// takes (`prepare`'s rooms) at its rung and at `of`.
#[test]
#[ignore = "the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS)"]
fn the_corpus_through_the_rungs() {
    use pp_words::RUNGS;
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let c = rerail_cpu_pp::native_cpu_pp::compile(program().unwrap()).unwrap();
    let by_buffer = std::cell::RefCell::new(Vec::<usize>::new());
    let bytes_at = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
        let (mut n, mut k) = (0usize, 0usize);
        let mut by = by_buffer.borrow_mut();
        let small = b.arena < Bounds::of(f).arena;
        let _ = pp_words::prepare(w, f, e, b, &mut |buf: pp_words::Buf| {
            n += buf.room.next_multiple_of(16);
            if small {
                // The files' buffers first (7 a file), then the machine's.
                let i = if k < 7 * f.len() { k % 7 } else { 7 + k - 7 * f.len() };
                if by.len() <= i {
                    by.resize(i + 1, 0);
                }
                by[i] += buf.room;
            }
            k += 1;
            0
        });
        n
    };
    let needed = std::cell::RefCell::new([0usize; RUNGS]);
    let reasons = std::cell::RefCell::new(std::collections::BTreeMap::<(usize, &'static str), usize>::new());
    let (at_rung, at_of) = (std::cell::RefCell::new(Vec::new()), std::cell::RefCell::new(Vec::new()));
    let laddered = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
        let mut r = 0;
        loop {
            let rb = Bounds::rung(f, r);
            let out = c.run(w, f, e, &rb);
            match &out {
                Err(x) if Bounds::is_room(x.why) && r + 1 < RUNGS => {
                    *reasons.borrow_mut().entry((r, x.why)).or_default() += 1;
                    r += 1;
                }
                _ => {
                    needed.borrow_mut()[r] += 1;
                    at_rung.borrow_mut().push(bytes_at(w, f, e, &rb));
                    at_of.borrow_mut().push(bytes_at(w, f, e, b));
                    return out;
                }
            }
        }
    };
    let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| c.run(w, f, e, b);
    let (mut same, mut differ) = (0usize, Vec::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            let src = std::fs::read(&p).unwrap();
            let files = [("main.c", src.as_slice())];
            let want = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &jit);
            let got = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &laddered);
            if got == want {
                same += 1;
            } else {
                differ.push(p.display().to_string());
            }
        }
    }
    let q = |v: &mut Vec<usize>, f: f64| {
        v.sort_unstable();
        v.get(((v.len() as f64 - 1.0) * f) as usize).copied().unwrap_or(0)
    };
    let (mut a, mut b) = (at_rung.borrow().clone(), at_of.borrow().clone());
    eprintln!("RUNGS the same {same}, differ {}: {:?}", differ.len(), differ.iter().take(5).collect::<Vec<_>>());
    eprintln!("RUNGS runs by the rung they needed: {:?}", needed.borrow());
    for ((r, why), n) in reasons.borrow().iter() {
        eprintln!("RUNGS   rung {r} refused {n}: {why}");
    }
    for (name, v) in [("at its rung", &mut a), ("at Bounds::of", &mut b)] {
        let sum: usize = v.iter().sum();
        eprintln!("RUNGS bytes a run {name}: median {} KB, p90 {} KB, p99 {} KB, max {} KB, all {} MB", q(v, 0.5) >> 10, q(v, 0.9) >> 10, q(v, 0.99) >> 10, q(v, 1.0) >> 10, sum >> 20);
    }
    eprintln!("RUNGS MB by buffer, at the rungs: {:?}", by_buffer.borrow().iter().map(|b| b >> 20).collect::<Vec<_>>());
    assert!(differ.is_empty());
}

/// What `corpus_on_metal` hands on for each chunk: its TUs (each one's path, and whether C++), and each
/// one's expansion or why it failed.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
type Chunk<'a> = dyn FnMut(&[(std::path::PathBuf, bool)], Vec<Result<Vec<Out>, String>>) + 'a;

/// The corpus's TUs (C and C++), `RERAIL_PP_CHUNK` at a time (512 by default), each chunk one
/// `expand_tus_with` whose rounds are the device's batches (0408, L4).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn corpus_on_metal(each: &mut Chunk<'_>, batch: &crate::expand::BatchRunner<'_>) {
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let mut tus = Vec::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        tus.extend(paths.into_iter().map(|p| (p, cpp)));
    }
    let chunk = std::env::var("RERAIL_PP_CHUNK").ok().and_then(|c| c.parse().ok()).unwrap_or(512);
    for part in tus.chunks(chunk) {
        let srcs: Vec<Vec<u8>> = part.iter().map(|(p, _)| std::fs::read(p).unwrap()).collect();
        let files: Vec<[(&str, &[u8]); 1]> = srcs.iter().map(|s| [("main.c", s.as_slice())]).collect();
        let mut io: Vec<_> = files.iter().map(|f| search(f, &[])).collect();
        let mut jobs: Vec<(&str, &mut _, bool)> = io.iter_mut().zip(part).map(|(io, (_, cpp))| ("main.c", io, *cpp)).collect();
        let got = crate::expand::expand_tus_with(&mut jobs, &INTERPRETED, batch);
        each(part, got);
    }
}

/// The device's batches' totals: GPU seconds, dispatches, the largest arena, the runs by rung.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[derive(Default)]
struct OnMetal {
    gpu: f64,
    dispatches: usize,
    arena: usize,
    rungs: [usize; pp_words::RUNGS],
    jobs: usize,
    batches: usize,
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn counted_batch<'a>(totals: &'a std::cell::RefCell<OnMetal>) -> impl Fn(&[crate::expand::Job<'_>]) -> Vec<Result<pp_words::Expanded, pp_words::Refused>> + 'a {
    move |jobs| {
        let ran = rerail_metal::pp_on::run_batch(crate::expand::ukl().unwrap(), jobs, jobs.len()).expect("the device's batch");
        let mut t = totals.borrow_mut();
        (t.gpu, t.dispatches, t.arena, t.jobs, t.batches) = (t.gpu + ran.gpu_seconds, t.dispatches + ran.dispatches, t.arena.max(ran.arena_bytes), t.jobs + jobs.len(), t.batches + 1);
        for (r, n) in ran.rungs.iter().enumerate() {
            t.rungs[r] += n;
        }
        ran.results
    }
}

/// The corpus on Metal (0408, L4): each TU the interpreter's, its rounds the device's batches.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
#[ignore = "the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS)"]
fn the_corpus_on_metal_is_the_interpreters() {
    let totals = std::cell::RefCell::new(OnMetal::default());
    let batch = counted_batch(&totals);
    let (mut same, mut refused, mut differ) = (0usize, std::collections::BTreeMap::<String, usize>::new(), Vec::new());
    corpus_on_metal(
        &mut |part, got| {
            for ((p, cpp), got) in part.iter().zip(got) {
                let src = std::fs::read(p).unwrap();
                let files = [("main.c", src.as_slice())];
                let want = expand_tu_with("main.c", &mut search(&files, &[]), *cpp, &INTERPRETED, &pp_words::run);
                match (&got, &want) {
                    (Err(a), Err(b)) if a == b => *refused.entry(b.chars().take(80).collect()).or_default() += 1,
                    _ if got == want => same += 1,
                    _ => differ.push(format!("{}: Metal {:?}, interpreter {:?}", p.display(), got.as_ref().map(Vec::len), want.as_ref().map(Vec::len))),
                }
            }
        },
        &batch,
    );
    let t = totals.borrow();
    eprintln!("ON_METAL the same {same}, refused by both {}, differ {}", refused.values().sum::<usize>(), differ.len());
    eprintln!("ON_METAL {} batches, {} jobs, {} dispatches, the largest arena {} MB, runs by rung {:?}, GPU {:.3} s", t.batches, t.jobs, t.dispatches, t.arena >> 20, t.rungs, t.gpu);
    for d in differ.iter().take(10) {
        eprintln!("  {d}");
    }
    assert!(differ.is_empty());
}

/// The corpus on Metal timed (0408, L4): the device's batches' GPU seconds, warmed first, against
/// the CPU's compiled words (`expand::native`: rustc's build where it is the process's words,
/// else Cranelift's) on the same jobs on this thread. A benchmark: through `quiet-run`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
#[ignore = "a benchmark of the corpus on Metal"]
fn the_corpus_on_metal_timed() {
    use rerail_compiler::native_cpu_codegen::retired;
    let totals = std::cell::RefCell::new(OnMetal::default());
    let cpu = std::cell::Cell::new((0u64, 0u64, std::time::Duration::ZERO));
    let device = counted_batch(&totals);
    let both = |jobs: &[crate::expand::Job<'_>]| {
        let ((i0, c0), t0) = (retired(), std::time::Instant::now());
        for &(w, f, e, b) in jobs {
            let _ = crate::expand::native(w, f, e, b);
        }
        let (t1, (i1, c1)) = (std::time::Instant::now(), retired());
        let (i, c, t) = cpu.get();
        cpu.set((i + i1 - i0, c + c1 - c0, t + (t1 - t0)));
        device(jobs)
    };
    // Warm: the pipeline compiled and the pool's blocks made.
    corpus_on_metal(&mut |_, _| {}, &both);
    *totals.borrow_mut() = OnMetal::default();
    cpu.set((0, 0, std::time::Duration::ZERO));
    let wall = std::time::Instant::now();
    corpus_on_metal(&mut |_, _| {}, &both);
    let wall = wall.elapsed();
    let t = totals.borrow();
    let (i, c, w) = cpu.get();
    eprintln!("ON_METAL_TIMED aot {:?}: {} batches, {} jobs, {} dispatches, runs by rung {:?}, the largest arena {} MB", crate::expand::aot_build().is_some(), t.batches, t.jobs, t.dispatches, t.rungs, t.arena >> 20);
    eprintln!("ON_METAL_TIMED GPU {:.1} ms; CPU words {:.1} M instructions, {:.1} M cycles, {:.1} ms; the whole pass {:.0} ms", t.gpu * 1e3, i as f64 / 1e6, c as f64 / 1e6, w.as_secs_f64() * 1e3, wall.as_secs_f64() * 1e3);
}
