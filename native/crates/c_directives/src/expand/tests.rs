use super::*;

/// The grammars by the interpreter (the same tapes as every backend's, `tokens::tests`), so a
/// test compiles no grammar through Cranelift.
const INTERPRETED: Grammars = Grammars { tokens: &tokens::interpreted, read: &crate::interpreted_read };

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

#[test]
fn the_programs_assemble() {
    for cpp in [false, true] {
        program(cpp).unwrap_or_else(|e| panic!("cpp {cpp}: {e}"));
    }
    assert!(program(true).unwrap().pp.suffix && !program(false).unwrap().pp.suffix);
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
        ),
    ),
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

/// The depths `#if`'s evaluator is compiled at in the tests (`compile_k`): none, one, and past
/// and within the examples' nesting.
const DEPTHS: [usize; 4] = [0, 1, 4, 16];

/// The compiled words (`native_cpu_pp`, decision 0348's stage 4b) give the interpreter's tokens
/// on every example and TU, in C and C++, `#if`'s evaluator at each of `DEPTHS`.
#[test]
fn the_compiled_words_are_the_interpreters() {
    use rerail_compiler::native_cpu_pp::compile_k;
    let dirs = [("inc", rerail_c_pp::files::DirKind::Angle), ("sys", rerail_c_pp::files::DirKind::System), ("sys2", rerail_c_pp::files::DirKind::System)];
    let mut tus: Vec<(String, Vec<(&str, &[u8])>)> = EXAMPLES.iter().map(|(n, s)| ((*n).to_owned(), vec![("main.c", s.as_bytes())])).collect();
    tus.extend(TUS.iter().map(|(n, f)| ((*n).to_owned(), f.iter().map(|f| (f.0, f.1.as_bytes())).collect())));
    let (mut same, mut differ) = (0, Vec::new());
    for (cpp, k) in [false, true].into_iter().flat_map(|c| DEPTHS.map(|k| (c, k))) {
        let p = program(cpp).unwrap();
        let c = compile_k(p, k).unwrap_or_else(|e| panic!("cpp {cpp} k {k}: {e:?}"));
        let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| c.run(w, f, e, b);
        for (name, files) in &tus {
            let want = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &pp_words::run);
            let got = expand_tu_with(files[0].0, &mut search(files, &dirs), cpp, &INTERPRETED, &jit);
            match got {
                got if got == want => same += 1,
                got => differ.push(format!("{name} (cpp {cpp}, k {k}): JIT {got:?}\n interpreter {want:?}")),
            }
        }
    }
    assert!(differ.is_empty(), "{} of {} differ:\n{}", differ.len(), same + differ.len(), differ.join("\n"));
}

/// The corpus's files, each a TU of its own: the compiled words' tokens equal the interpreter's
/// on every file the grammars read, and the TUs one refuses the other refuses for the same reason.
#[test]
#[ignore = "reads the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS)"]
fn the_corpus_compiled_is_the_interpreters() {
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let (mut same, mut out, mut differ) = (0, std::collections::BTreeMap::<String, usize>::new(), Vec::new());
    let (mut t_words, mut t_jit) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    let depths = std::cell::RefCell::new(std::collections::BTreeMap::<usize, usize>::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        // Each TU at its files' depth (`Compiled::depth`).
        let c = Compiled::new();
        let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
            *depths.borrow_mut().entry(Compiled::depth(f)).or_default() += 1;
            c.run(w, f, e, b)
        };
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
    eprintln!("the runs by depth: {:?}", depths.borrow());
    eprintln!("the same {same}, refused by both {}, differ {}; the expanded TUs: interpreter {t_words:?}, JIT {t_jit:?} (the grammars' reads included)", out.values().sum::<usize>(), differ.len());
    for (why, n) in &out {
        eprintln!("  {n:5} {why}");
    }
    for d in differ.iter().take(15) {
        eprintln!("  {d}");
    }
    assert!(differ.is_empty());
}

/// The compiled words' shape (decision 0348, stage 4d), at every depth: no call but of their
/// own functions (the large words', `OUTLINE`, and past depth 0 every one of `#if`'s `EXPR`
/// words), no indirect or tail call, no function over 128 KB, and each depth's returns their own
/// stated dispatches (`Step::Return`).
#[test]
fn the_compiled_words_have_their_shape() {
    use rerail_compiler::native_cpu_pp::{MAX_K, compile_k};
    use rerail_compiler::native_cpu_shape::Step;
    for cpp in [false, true] {
        let p = program(cpp).unwrap();
        let returns = |k: usize| compile_k(p, k).unwrap().heads.iter().filter(|&&h| h == Step::Return).count();
        let (r0, r1) = (returns(0), returns(1));
        let at0 = compile_k(p, 0).unwrap().code_bytes.len();
        for k in [0, 1, 2, 4, 8, MAX_K] {
            let c = compile_k(p, k).unwrap_or_else(|e| panic!("cpp {cpp} k {k}: {e:?}"));
            eprintln!("cpp {cpp} k {k}: code {:?} bytes, {} CLIF instructions", c.code_bytes, c.clif.lines().filter(|l| l.starts_with("    ") && !l.trim_end().ends_with(':')).count());
            if k == 0 {
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
            }
            let functions = c.clif.matches("function ").count();
            assert!(!c.clif.contains("call_indirect") && !c.clif.contains("return_call"), "cpp {cpp} k {k}");
            let calls = c.clif.lines().filter(|l| l.contains(" = call ") || l.trim_start().starts_with("call ")).count();
            assert_eq!(functions, c.code_bytes.len(), "cpp {cpp} k {k}");
            assert!(calls > 0 && (k == 0 || functions > at0), "cpp {cpp} k {k}");
            assert!(c.code_bytes.iter().all(|&b| b <= 128 << 10), "cpp {cpp} k {k}: {:?}", c.code_bytes);
            // Each depth past 0 adds the evaluator's `RETURN`s' dispatches once more.
            assert_eq!(c.heads.iter().filter(|&&h| h == Step::Return).count(), r0 + k * (r1 - r0), "cpp {cpp} k {k}");
        }
    }
}

/// The corpus's TUs expanded by the words' interpreter, by the compiled words (each TU at its
/// depth) and by the lean pass: each's instructions retired, cycles and wall time on this
/// thread, the runs warmed first (every depth compiled). The words' runs are their runs alone
/// (the grammars' reads are tier 1's and mode (a)'s); the lean pass's lexes too. A benchmark:
/// through `scripts/dev/quiet-run`, at `--profile profiling`.
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
    let mut tus = 0;
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        let srcs: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let c = Compiled::new();
        for pass in 0..2 {
            let on = Cell::new(pass == 1);
            let run_words = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
                let mut r = None;
                if on.get() { timed(&words, &mut || r = Some(pp_words::run(w, f, e, b))) } else { r = Some(pp_words::run(w, f, e, b)) }
                r.unwrap()
            };
            let run_jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| {
                let mut r = None;
                if on.get() { timed(&jit, &mut || r = Some(c.run(w, f, e, b))) } else { r = Some(c.run(w, f, e, b)) }
                r.unwrap()
            };
            for src in &srcs {
                let files = [("main.c", src.as_slice())];
                let want = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &run_words);
                if want.is_err() {
                    continue;
                }
                let got = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &run_jit);
                assert_eq!(got, want);
                if pass == 1 {
                    timed(&lean_pass, &mut || {
                        std::hint::black_box(lean(src, cpp));
                    });
                    tus += 1;
                }
            }
        }
    }
    for (name, c) in [("the words' interpreter", words.get()), ("the compiled words", jit.get()), ("the lean pass (lexing too)", lean_pass.get())] {
        eprintln!("{name:28} {:>8.1} M instructions {:>8.1} M cycles {:>10.3?}", c.instructions as f64 / 1e6, c.cycles as f64 / 1e6, c.wall);
    }
    eprintln!("over {tus} TUs");
}

/// `if_depth` on lines whose calls are counted by hand on `expand.lab`'s `cond`.
#[test]
fn an_ifs_depth_is_estimated_from_above() {
    for (line, want) in [("1", 3), ("-1", 4), ("(1)", 7), ("1 + 2", 5), ("1 + 2 * 3", 7), ("a && (b || c)", 11), ("1 ? 2 : 3", 5), ("((1))", 11), ("!defined X", 4), ("1 ? 2 : 3 ? 4 : 5", 6), ("(1, 2)", 7), ("1 * 2 + 3", 7)] {
        let src = format!("#if {line}\n#endif\n");
        let toks = tokens::interpreted(src.as_bytes(), false).unwrap();
        let read = crate::interpreted_read(src.as_bytes()).unwrap();
        let tape = tape(src.as_bytes(), &toks, 0, &mut Vec::new(), u32::MAX - 1);
        assert_eq!(if_depth(&read, src.as_bytes(), &tape), want, "{line}");
    }
}

/// `#if`'s primaries and operators, the JIT's against the interpreter's, value by value: each
/// case's 64 bits, whether it is unsigned, and whether it is an error, each an `#if` of its own.
#[test]
fn the_compiled_ifs_have_the_interpreters_values() {
    use rerail_compiler::native_cpu_pp::compile_k;
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
    for cpp in [false, true] {
        let p = program(cpp).unwrap();
        let compiled: Vec<_> = DEPTHS.iter().map(|&k| compile_k(p, k).unwrap_or_else(|e| panic!("cpp {cpp} k {k}: {e:?}"))).collect();
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
            for (c, k) in compiled.iter().zip(DEPTHS) {
                let jit = |w: &Words, f: &[File], e: &pp_words::Env, b: &Bounds| c.run(w, f, e, b);
                let got = expand_tu_with("main.c", &mut search(&files, &[]), cpp, &INTERPRETED, &jit).unwrap_or_else(|e| panic!("{case} (cpp {cpp}, k {k}): {e}"));
                if got != want {
                    differ.push(format!("{case} (k {k}): interpreter {} JIT {}", words(&want), words(&got)));
                }
            }
        }
        assert!(differ.is_empty(), "cpp {cpp}:\n{}", differ.join("\n"));
    }
    eprintln!("the token grammar refuses {unlexed:?}");
}
