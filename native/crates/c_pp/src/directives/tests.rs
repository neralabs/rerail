use super::*;

fn c() -> Options {
    Options::c(17, false)
}

fn s<'a>(d: &'a Directives, sp: Span) -> &'a str {
    std::str::from_utf8(d.text(sp)).unwrap()
}

fn kinds(d: &Directives) -> Vec<&'static str> {
    d.all.iter().map(|x| x.kind.name()).collect()
}

#[test]
fn defines_object_and_function_like() {
    let src = b"#define A 1 + 2\n#define F(x, y) x ## y\n#define G (x)\n#define V(a, ...) f(a, __VA_ARGS__)\n#define N(args...) g(args)\n#define E\n#define H() h\n";
    let d = scan(src, c());
    assert_eq!(kinds(&d), ["define"; 7]);
    let got: Vec<_> = d
        .defines
        .iter()
        .map(|m| (s(&d, m.name), m.fnlike, m.params.iter().map(|p| s(&d, *p)).collect::<Vec<_>>(), m.variadic, s(&d, m.body)))
        .collect();
    use Variadic::*;
    assert_eq!(
        got,
        [
            ("A", false, vec![], No, "1 + 2"),
            ("F", true, vec!["x", "y"], No, "x ## y"),
            // A blank before `(`: object-like, the `(` the body's.
            ("G", false, vec![], No, "(x)"),
            ("V", true, vec!["a"], Standard, "f(a, __VA_ARGS__)"),
            ("N", true, vec!["args"], Named, "g(args)"),
            ("E", false, vec![], No, ""),
            ("H", true, vec![], No, "h"),
        ]
    );
    assert!(d.problems.is_empty());
}

#[test]
fn malformed_defines_are_problems() {
    let d = scan(b"#define\n#define 1\n#define F(a,\n#define G(a,)\n#define H(a b)\n", c());
    assert_eq!(d.problems, [Problem::Malformed(0), Problem::Malformed(1), Problem::Malformed(2), Problem::Malformed(3), Problem::Malformed(4)]);
}

#[test]
fn includes_by_form() {
    let src = b"#include <stdio.h>\n#include \"a b.h\"\n#include_next <x.h>\n#import \"m.h\"\n#include HEADER(x)\n#embed \"data.bin\" limit(4)\n#embed DATA limit(4)\n";
    let d = scan(src, c());
    let got: Vec<_> = d.includes.iter().map(|x| (x.i, x.kind.name(), x.form, s(&d, x.spelled))).collect();
    use Form::*;
    assert_eq!(
        got,
        [
            (0, "include", Angle, "<stdio.h>"),
            (1, "include", Quote, "\"a b.h\""),
            (2, "include_next", Angle, "<x.h>"),
            (3, "import", Quote, "\"m.h\""),
            (4, "include", Macro, "HEADER(x)"),
            (5, "embed", Quote, "\"data.bin\""),
            (6, "embed", Macro, "DATA"),
        ]
    );
}

#[test]
fn conditionals_chain_and_nest() {
    let src = b"#if A\n#ifdef B\n#define X\n#elifndef C\n#endif\n#elif D > 1\n#else\n#undef X\n#endif\n#endif\n#else\n";
    let d = scan(src, c());
    assert_eq!(kinds(&d), ["if", "ifdef", "define", "elifndef", "endif", "elif", "else", "undef", "endif", "endif", "else"]);
    let chain: Vec<_> = d.conds.iter().map(|x| (x.i, s(&d, x.expr), x.next)).collect();
    assert_eq!(
        chain,
        [(0, "A", Some(5)), (1, "B", Some(3)), (3, "C", Some(4)), (4, "", None), (5, "D > 1", Some(6)), (6, "", Some(8)), (8, "", None), (9, "", None), (10, "", None)]
    );
    let parents: Vec<_> = d.all.iter().map(|x| x.parent).collect();
    assert_eq!(parents, [None, Some(0), Some(1), Some(0), Some(0), None, None, Some(6), None, None, None]);
    assert_eq!(d.problems, [Problem::Unmatched(9), Problem::Unmatched(10)]);
    let open = scan(b"#if 1\n#else\n#elif 2\n", c());
    assert_eq!(open.problems, [Problem::AfterElse(2), Problem::Unterminated(0)]);
}

#[test]
fn pragmas_lines_and_texts() {
    let src = b"#pragma once\n#pragma GCC system_header\n#pragma push_macro(\"X\")\n#line 42 \"f.c\"\n# 7 \"g.h\" 1 3\n#line N\n#error no \"way\"\n#warning w\n#ident \"v1\"\n#foo bar\n#\n#!x\n";
    let d = scan(src, c());
    assert_eq!(kinds(&d), ["pragma", "pragma", "pragma", "line", "line_marker", "line", "error", "warning", "ident", "unknown", "null", "unknown"]);
    let p: Vec<_> = d.pragmas.iter().map(|x| (s(&d, x.first), s(&d, x.second), s(&d, x.text))).collect();
    assert_eq!(p, [("once", "", "once"), ("GCC", "system_header", "GCC system_header"), ("push_macro", "", "push_macro(\"X\")")]);
    let l: Vec<_> = d.lines.iter().map(|x| (x.i, x.n, s(&d, x.file))).collect();
    assert_eq!(l, [(3, Some(42), "\"f.c\""), (4, Some(7), "\"g.h\""), (5, None, "")]);
    let t: Vec<_> = d.texts.iter().map(|x| (x.i, s(&d, x.text))).collect();
    assert_eq!(t, [(6, "no \"way\""), (7, "w"), (8, "\"v1\""), (9, "bar"), (11, "x")]);
}

#[test]
fn only_a_lines_first_token_starts_one() {
    // A `#` after a token, in a string or in a comment is none. After a comment that spans lines it
    // is one where its line started before the comment (clang's reading: a line end in a comment
    // starts no line, nor ends one).
    let src = b"a # define X\n\"#define Y\"\n/* #define Z */ x\n/*\n*/ #define W\n  /**/ #define V 1\n%:define U\n#def\\\nine T\n";
    let d = scan(src, c());
    let names: Vec<_> = d.defines.iter().map(|m| s(&d, m.name)).collect();
    assert_eq!(names, ["W", "V", "U", "T"]);
    // Lines and spans are the file's.
    let lines: Vec<_> = d.all.iter().map(|x| x.line).collect();
    assert_eq!(lines, [5, 6, 7, 8]);
    let (a, b) = d.file(d.all[3].span);
    assert_eq!(&src[a as usize..b as usize], b"#def\\\nine T");
}

#[test]
fn a_define_goes_on_past_a_comment_over_lines() {
    let d = scan(b"#define A 1 /* two\nlines */ + 2\nint x;\nx /*\n*/ #define Z 3\n", c());
    assert_eq!(s(&d, d.defines[0].body), "1 /* two\nlines */ + 2");
    // Nor does a line end in a comment start a line (clang: `x #define Z 3` is text).
    assert_eq!(d.all.len(), 1);
}

#[test]
fn guards_are_clangs() {
    let g = |src: &[u8]| {
        let d = scan(src, c());
        d.guard.map(|(i, n)| (i, s(&d, n).to_string()))
    };
    assert_eq!(g(b"// c\n#ifndef H\n#define H\nint x;\n#if A\n#endif\n#endif\n// end\n"), Some((0, "H".into())));
    assert_eq!(g(b"#if !defined(H)\n#define H\n#endif\n"), Some((0, "H".into())));
    assert_eq!(g(b"#if !defined H\n#endif\n"), Some((0, "H".into())));
    assert_eq!(g(b"int y;\n#ifndef H\n#endif\n"), None);
    assert_eq!(g(b"#ifndef H\n#endif\nint y;\n"), None);
    assert_eq!(g(b"#ifndef H\n#else\n#endif\n"), None);
    assert_eq!(g(b"#ifndef H\n#endif\n#define Q\n"), None);
    assert_eq!(g(b"#define Q\n#ifndef H\n#endif\n"), None);
    assert_eq!(g(b"#if !defined(H) && 1\n#endif\n"), None);
    assert_eq!(g(b"#ifndef H\n"), None);
}

/// The directives' starts, kinds and lines, by a byte-at-a-time reading of the phase-2 text.
fn reference(d: &Directives, o: Options) -> Vec<(u32, &'static str, u32)> {
    let t: &[u8] = &d.p2.text;
    let at = |i: usize| t.get(i).copied().unwrap_or(0);
    let mut out = Vec::new();
    let (mut i, mut bol) = (0, true);
    while i < t.len() {
        let ch = at(i);
        if ch == b'\n' {
            bol = true;
            i += 1;
        } else if matches!(ch, b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
            i += 1;
        } else if ch == b'/' && at(i + 1) == b'*' {
            i = (i + 2..t.len()).find(|&k| at(k) == b'*' && at(k + 1) == b'/').map_or(t.len(), |k| k + 2);
        } else if ch == b'/' && at(i + 1) == b'/' {
            i = (i..t.len()).find(|&k| at(k) == b'\n').unwrap_or(t.len());
        } else if ch == b'#' && at(i + 1) == b'#' || t[i..].starts_with(b"%:%:") {
            // `##` starts no directive.
            bol = false;
            i += if ch == b'#' { 2 } else { 4 };
        } else if bol && (ch == b'#' || ch == b'%' && at(i + 1) == b':') {
            let s = i;
            i += if ch == b'#' { 1 } else { 2 };
            bol = false;
            // The name: blanks and comments on the line passed.
            loop {
                if matches!(at(i), b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
                    i += 1;
                } else if at(i) == b'/' && at(i + 1) == b'*' {
                    i = (i + 2..t.len()).find(|&k| at(k) == b'*' && at(k + 1) == b'/').map_or(t.len(), |k| k + 2);
                } else {
                    break;
                }
            }
            let e = (i..t.len()).find(|&k| !(at(k).is_ascii_alphanumeric() || at(k) == b'_')).unwrap_or(t.len());
            let kind = if i >= t.len() || at(i) == b'\n' || at(i) == b'/' && at(i + 1) == b'/' {
                "null"
            } else if at(i).is_ascii_digit() {
                "line_marker"
            } else {
                match NAMES[1..].iter().position(|n| *n == &t[i..e]) {
                    Some(k) => KINDS[k].name(),
                    _ => "unknown",
                }
            };
            let line = 1 + count_lines(d, s);
            out.push((s as u32, kind, line));
        } else if ch == b'"' || ch == b'\'' {
            bol = false;
            let mut k = i + 1;
            let close = loop {
                match at(k) {
                    _ if k >= t.len() => break None,
                    b'\n' => break None,
                    b'\\' => k += 2,
                    c if c == ch => break Some(k + 1),
                    _ => k += 1,
                }
            };
            i = close.unwrap_or(i + 1);
        } else if ch.is_ascii_alphanumeric() || ch == b'_' {
            // A word or a pp-number, whose `'` before a digit or letter is a digit separator.
            bol = false;
            let num = ch.is_ascii_digit();
            i += 1;
            loop {
                let c = at(i);
                if c.is_ascii_alphanumeric() || c == b'_' || num && c == b'.' {
                    i += 1;
                } else if num && o.digit_separators && c == b'\'' && (at(i + 1).is_ascii_alphanumeric() || at(i + 1) == b'_') {
                    i += 2;
                } else if num && matches!(c, b'+' | b'-') && matches!(at(i - 1), b'e' | b'E' | b'p' | b'P') {
                    i += 1;
                } else {
                    break;
                }
            }
        } else {
            bol = false;
            i += 1;
        }
    }
    out
}

/// The `\n`s of the file before the phase-2 position `s`.
fn count_lines(d: &Directives, s: usize) -> u32 {
    let src_end = d.p2.orig(s);
    // The file is read again from the phase-2 text and the removed ranges: the reference counts
    // the file's own bytes.
    FILE.with(|f| f.borrow()[..src_end].iter().filter(|&&b| b == b'\n').count() as u32)
}

thread_local! {
    static FILE: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn check(src: &[u8], opts: Options) {
    FILE.with(|f| *f.borrow_mut() = src.to_vec());
    let d = scan(src, opts);
    let got: Vec<_> = d.all.iter().map(|x| (x.span.start, x.kind.name(), x.line)).collect();
    assert_eq!(got, reference(&d, opts), "{:?}", String::from_utf8_lossy(src));
}

#[test]
fn scan_reads_as_the_byte_reading_does() {
    let pieces = [
        "#", "%:", "define", "if", "endif", "include", "line", " ", "\t", "\n", "\n", "\n", "/*x*/", "/*\n*/", "//c", "\"s#\"", "'#'", "\\\n", "a", "1", "<x.h>", "(", ")", "elif", "'", "\"", "??=", "1e",
    ];
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..2000 {
        let mut s = String::new();
        for _ in 0..16 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.push_str(pieces[(x % pieces.len() as u64) as usize]);
        }
        check(s.as_bytes(), c());
        check(s.as_bytes(), Options::c(23, true));
    }
}

/// Every file of the C and C++ corpora (where they are here), against the byte reading.
#[test]
fn the_corpora_read_as_the_byte_reading_does() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let root = std::path::Path::new(&home).join("orca/rerail-shared/lean-corpus");
    let mut files = 0;
    let mut directives = 0;
    let (mut guards, mut problems) = (0, std::collections::BTreeMap::<String, usize>::new());
    for (dir, opts) in [("c", Options::c(17, true)), ("cpp", Options::cpp(20, true))] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        for e in rd.flatten() {
            let Ok(src) = std::fs::read(e.path()) else { continue };
            FILE.with(|f| *f.borrow_mut() = src.clone());
            let d = scan(&src, opts);
            // The reference reads no raw strings: files with one are checked for their count only.
            if !opts.cpp || !src.windows(2).any(|w| w == b"R\"") {
                let got: Vec<_> = d.all.iter().map(|x| (x.span.start, x.kind.name(), x.line)).collect();
                assert_eq!(got, reference(&d, opts), "{}", e.path().display());
            }
            files += 1;
            directives += d.all.len();
            guards += usize::from(d.guard.is_some());
            for p in &d.problems {
                let k = format!("{p:?}");
                *problems.entry(k[..k.find('(').unwrap_or(k.len())].to_string()).or_default() += 1;
            }
        }
    }
    eprintln!("{files} files, {directives} directives, {guards} guarded, problems {problems:?}");
}
