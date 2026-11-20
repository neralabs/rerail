use super::*;

/// The lean scan's rows of `src`: its reference.
fn lean(src: &[u8], cpp: bool) -> Vec<Row> {
    let opts = if cpp { rerail_c_pp::Options::cpp(17, true) } else { rerail_c_pp::Options::c(17, true) };
    let d = rerail_c_pp::directives::scan(src, opts);
    d.all
        .iter()
        .map(|x| {
            let (start, end) = d.file(x.span);
            Row { kind: x.kind.name(), start, end, parent: x.parent }
        })
        .collect()
}

/// The lean scan's operands of `src`, as `operands` gives them: an empty span is none, a
/// function-like macro's `Params` at 0..0 (the lean scan keeps only that it is one), a line's
/// `Number` its value at `start`.
fn lean_operands(src: &[u8], cpp: bool) -> Vec<Operand> {
    let opts = if cpp { rerail_c_pp::Options::cpp(17, true) } else { rerail_c_pp::Options::c(17, true) };
    let d = rerail_c_pp::directives::scan(src, opts);
    let mut out = Vec::new();
    let mut put = |i: u32, what: What, s: rerail_c_pp::directives::Span| {
        if !s.is_empty() {
            let (start, end) = d.file(s);
            out.push(Operand { i, what, start, end });
        }
    };
    use rerail_c_pp::directives::{Form, Variadic};
    for m in &d.defines {
        put(m.i, What::Name, m.name);
        for p in &m.params {
            put(m.i, What::Param, *p);
        }
        put(m.i, What::Body, m.body);
    }
    for (i, n) in &d.undefs {
        put(*i, What::Name, *n);
    }
    for c in &d.includes {
        put(c.i, match c.form { Form::Quote => What::Quote, Form::Angle => What::Angle, Form::Macro => What::Macro }, c.spelled);
    }
    for c in &d.conds {
        put(c.i, What::Expr, c.expr);
    }
    for p in &d.pragmas {
        put(p.i, What::First, p.first);
        put(p.i, What::Second, p.second);
        put(p.i, What::Text, p.text);
    }
    for l in &d.lines {
        put(l.i, What::File, l.file);
        put(l.i, What::Rest, l.rest);
    }
    for t in &d.texts {
        put(t.i, What::Text, t.text);
    }
    // The operands without bytes in the lean scan's rows.
    for m in &d.defines {
        if m.fnlike {
            out.push(Operand { i: m.i, what: What::Params, start: 0, end: 0 });
        }
        match m.variadic {
            Variadic::No => {}
            Variadic::Standard => out.push(Operand { i: m.i, what: What::Variadic, start: 0, end: 0 }),
            Variadic::Named => out.push(Operand { i: m.i, what: What::NamedVariadic, start: 0, end: 0 }),
        }
    }
    for l in &d.lines {
        if let Some(n) = l.n {
            out.push(Operand { i: l.i, what: What::Number, start: n as u32, end: 0 });
        }
    }
    out.sort();
    out
}

/// `operands` made comparable with `lean_operands`: `Params` and the variadics at 0..0, a
/// `Number` its value (none where it does not parse).
fn normal(src: &[u8], ops: &[Operand]) -> Vec<Operand> {
    let mut out: Vec<Operand> = ops
        .iter()
        .filter_map(|o| match o.what {
            What::Params | What::Variadic | What::NamedVariadic => Some(Operand { start: 0, end: 0, ..o.clone() }),
            What::Number => std::str::from_utf8(&src[o.start as usize..o.end as usize]).ok()?.parse::<u64>().ok().map(|n| Operand { start: n as u32, end: 0, ..o.clone() }),
            _ => Some(o.clone()),
        })
        .collect();
    out.sort();
    out
}

/// The lean scan's chains: each conditional and the next of its chain.
fn lean_next(src: &[u8], cpp: bool) -> Vec<(u32, u32)> {
    let opts = if cpp { rerail_c_pp::Options::cpp(17, true) } else { rerail_c_pp::Options::c(17, true) };
    let mut out: Vec<(u32, u32)> = rerail_c_pp::directives::scan(src, opts).conds.iter().filter_map(|c| Some((c.i, c.next?))).collect();
    out.sort();
    out
}

/// What `read` gives, the lean scan's: rows (with parents), operands, chains.
fn same(r: &Read, src: &[u8], cpp: bool) -> bool {
    let mut next = r.next.clone();
    next.sort();
    r.rows == lean(src, cpp) && normal(src, &r.operands) == lean_operands(src, cpp) && next == lean_next(src, cpp) && Facts::of(r, src) == lean_facts(src, cpp)
}

fn lean_facts(src: &[u8], cpp: bool) -> Facts {
    Facts::lean(src, if cpp { rerail_c_pp::Options::cpp(17, true) } else { rerail_c_pp::Options::c(17, true) })
}

fn compiled_or_panic() -> &'static Compiled {
    compiled().unwrap_or_else(|e| panic!("the grammar compiles: {e}"))
}

/// Each case reads as the lean scan reads it: kinds, bytes and operands.
#[test]
fn directives_read_as_the_lean_scan_reads_them() {
    let c = compiled_or_panic();
    let cases: &[&str] = &[
        "#define X 1\n",
        "#include <stdio.h>\nint main() { return 0; }\n",
        "  #  ifdef A  \n#endif\n",
        "#\n# 33 \"f.c\" 1\n#foo bar\n#!x\n",
        "/* a */ #define A /* b */ 2 // c\n",
        "#define LONG(a, b) \\\n    ((a) + \\\n     (b))\nint x;\n",
        "int s = \"#define not\"; // #define not\n/*\n#define not\n*/\n#pragma once\n",
        "#error \"don't\"\n#if 0\n#else\n#endif",
        "#include \"a.h\"\n#include_next <b.h>\n#import \"c.h\"\n#line 3 \"x\"\n#undef X\n",
        "x = a / b; /* c */\n#define Y\n",
        "#define S(x) #x\n#define C(a, b) a ## b\n",
        "\u{feff}#pragma once\n#include \"a.h\"\n",
        "#define F(a, b) a + b\n#define G() 1\n#define H (x)\n#define V(...) __VA_ARGS__\n#define W(a, ...) a\n#define N(args...) f(args)\n#define E\n",
        "#undef X junk\n#undef\n#include M(x)\n#embed <a.bin> limit(4)\n#include <sys/a.h> // c\n",
        "#ifdef A\n#elif defined(B) && C > 1\n#else x\n#endif // A\n#if\n#endif\n",
        "#pragma GCC diagnostic push\n#pragma once\n#pragma 1 x\n#pragma\n#pragma omp(x)\n",
        "#line 42\n#line 7 \"f.c\"\n# 33 \"g.c\" 1 3\n#line X \"h.c\"\n#line\n",
        "#error a b\n#warning\n#ident \"x\"\n#foo bar baz\n#! x\n#define include 1\n#undef define\n",
        "#define LONG(a, \\\n  b) \\\n  ((a) \\\n + (b))\n",
        "#ifndef G\n#define G\n#if A\n#define X\n#elif B\n#ifdef C\n#endif\n#else\n#undef X\n#endif\nint x;\n#endif\n",
        "#if 0\n#if 1\n#elifdef Z\n#elifndef Y\n#else\n#endif\n#endif\n#define Q #x ## y\n# if 1\n#  endif\n",
        "#endif\n#else\n#define A\n",
        "#ifndef G_H\n#define G_H\nint x;\n#endif\n",
        "// c\n#if !defined(G)\n#endif /* G */\n",
        "#if ! defined G\n#endif\n\n",
        "int a;\n#ifndef G\n#endif\n",
        "#ifndef G\n#endif\nint a;\n",
        "#ifndef G\n#else\n#endif\n#define\n#undef 1\n#include\n#if A\n#else\n#elif B\n#endif\n",
        "#define A(x) x \\\n  + 1\n#pragma a \\\n b\n",
        "#ifndef G\r\n#define G 1 \r\n#define L(a) a \\\r\n + 1\r\n#include \"x.h\"\r\n// c\r\n#pragma once\r\n#endif\r\n",
        "#error done\r\n#define S \"a\"\r\nint x;\r\n",
        "auto s = R\"(\n#define NOT_ONE\n)\";\n#define A u8R\"(x)\" LR\"()\"\n",
        "#ifndef G\n#define G\nconst char* s = R\"(\"\\)\";\n#endif\n",
        "#define S R\"(two\nlines)\" after\n#define FOOR\n",
    ];
    for src in cases {
        let b = src.as_bytes();
        let got = c.read(b).unwrap_or_else(|| panic!("refused: {src:?}"));
        let text = || image_text(&c.image(b).unwrap()).unwrap();
        assert_eq!(got.rows, lean(b, false), "{src:?}\n{}", text());
        assert_eq!(normal(b, &got.operands), lean_operands(b, false), "{src:?}\n{}", text());
        let mut next = got.next.clone();
        next.sort();
        assert_eq!(next, lean_next(b, false), "{src:?}\n{}", text());
        assert_eq!(Facts::of(&got, b), lean_facts(b, false), "{src:?}\n{}", text());
        assert_eq!(c.interpreter_read(b).as_ref(), Some(&got), "the interpreter, {src:?}");
    }
    #[cfg(target_os = "macos")]
    {
        let files: Vec<&[u8]> = cases.iter().map(|s| s.as_bytes()).collect();
        let metal = c.metal_read(&files).unwrap_or_else(|e| panic!("the device runs the batch: {e:?}"));
        for (src, m) in cases.iter().zip(metal) {
            assert_eq!(m, c.read(src.as_bytes()), "Metal, {src:?}");
        }
    }
}

/// What the grammar does not read is refused, never read wrong.
#[test]
fn what_the_grammar_does_not_read_is_refused() {
    let c = compiled_or_panic();
    for src in ["#if A\n#define X\n", "#define X 1\rx\n", "auto s = R\"x(x)x\";\n", "auto s = R\"(x\n", "auto s = R\"(a\\\nb)\";\n", "int n = 1'000;\n", "#define X \"open\n", "/* open\n", "??=define X\n", "%:define X\n", "#error don't\n"] {
        assert!(c.read(src.as_bytes()).is_none(), "{src:?}");
    }
}

/// The corpus (`~/orca/rerail-shared/lean-corpus/{c,cpp}`, or `RERAIL_C_CORPUS`): each file is
/// read the same, or refused; none differs. Ignored: it reads 3,000 files.
#[test]
#[ignore]
fn the_corpus_reads_the_same_or_is_refused() {
    let c = compiled_or_panic();
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let (mut same_n, mut refused, mut differ) = (0, 0, Vec::new());
    // The other backends', against Cranelift's: files whose rows differ.
    let (mut interp, mut metal) = (Vec::new(), Vec::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            let src = std::fs::read(&p).unwrap();
            let cl = c.read(&src);
            if c.interpreter_read(&src) != cl {
                interp.push(p.display().to_string());
            }
            #[cfg(target_os = "macos")]
            if c.metal_read(&[&src]).unwrap_or_else(|e| panic!("the device runs {}: {e:?}", p.display()))[0] != cl {
                metal.push(p.display().to_string());
            }
            match cl {
                None => refused += 1,
                Some(r) if same(&r, &src, cpp) => same_n += 1,
                Some(r) => {
                    let (rows, want) = (r.rows.clone(), lean(&src, cpp));
                    let at = rows.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(rows.len().min(want.len()));
                    let mut next = r.next.clone();
                    next.sort();
                    if rows != want {
                        differ.push(format!("{}: at {at}: {:?} against {:?}", p.display(), rows.get(at), want.get(at)));
                    } else if next != lean_next(&src, cpp) {
                        differ.push(format!("{}: chains {:?} against {:?}", p.display(), next.iter().take(8).collect::<Vec<_>>(), lean_next(&src, cpp).iter().take(8).collect::<Vec<_>>()));
                    } else {
                        let (ops, want) = (normal(&src, &r.operands), lean_operands(&src, cpp));
                        let at = ops.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(ops.len().min(want.len()));
                        let text = |o: Option<&Operand>| o.map(|o| format!("{o:?} {:?}", String::from_utf8_lossy(&src[o.start.min(o.end) as usize..o.end as usize])));
                        if ops != want {
                            differ.push(format!("{}: operand at {at}: {:?} against {:?}", p.display(), text(ops.get(at)), text(want.get(at))));
                        } else {
                            let (got, want) = (Facts::of(&r, &src), lean_facts(&src, cpp));
                            let first = |a: &str, b: &str| {
                                let at = a.bytes().zip(b.bytes()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
                                let from = at.saturating_sub(120);
                                format!("{} against {}", &a[from..(at + 120).min(a.len())], &b[from..(at + 120).min(b.len())])
                            };
                            differ.push(format!("{}: facts: {}", p.display(), first(&format!("{got:?}"), &format!("{want:?}"))));
                        }
                    }
                }
            }
        }
    }
    eprintln!("the same {same_n}, refused {refused}, differ {}; the interpreter differs on {}, Metal on {}", differ.len(), interp.len(), metal.len());
    for p in interp.iter().chain(&metal).take(10) {
        eprintln!("  backend: {p}");
    }
    for d in differ.iter().take(20) {
        eprintln!("  {d}");
    }
    assert!(differ.is_empty() && interp.is_empty() && metal.is_empty());
}

/// A file past 64 KiB: its tape needs 4-byte columns (the root's tail from its last directive to
/// the file's end, and offsets from a block's base), so the narrow parser refuses it and the wide
/// one reads it, on every backend.
#[test]
fn a_file_past_64_kib_is_read_at_wide_columns() {
    let c = compiled_or_panic();
    for (head, tail) in [("#define A 1\n", ""), ("", "#include <x.h>\n"), ("#if X\n", "#endif\n")] {
        let src = format!("{head}{}{tail}", "int x;\n".repeat(12_000));
        let got = c.read(src.as_bytes()).unwrap_or_else(|| panic!("refused: {head:?} … {tail:?}"));
        assert_eq!(got.rows, lean(src.as_bytes(), false));
        assert!(same(&got, src.as_bytes(), false));
        assert_eq!(c.interpreter_read(src.as_bytes()), Some(got.clone()), "the interpreter");
        #[cfg(target_os = "macos")]
        assert_eq!(c.metal_read(&[src.as_bytes()]).unwrap_or_else(|e| panic!("the device runs it: {e:?}"))[0], Some(got), "Metal");
    }
}
