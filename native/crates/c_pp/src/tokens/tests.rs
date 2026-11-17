use super::*;
use crate::files::Search;
use crate::stream::{Config, Pp};
use rerail_c_lean::Dialect;
use rerail_c_lean::pp::Options;

/// `main` preprocessed (its headers from `files`) into an `Expanded`.
fn expand(files: &[(&str, Vec<u8>)], main: &str, opts: Options) -> Expanded {
    let m: std::collections::HashMap<String, Vec<u8>> = files.iter().cloned().map(|(p, s)| (p.to_string(), s)).collect();
    let mut io = Search::new(Vec::new(), move |p: &str| m.get(p).cloned());
    let mut sink = TokenSink::new(opts.cpp);
    let mut pp = Pp::new(Config::new(opts), &mut io, &mut sink);
    assert!(pp.run(main));
    drop(pp);
    sink.finish()
}

/// The parse of the given tokens against the parse of their text: the same tape, or the same
/// refusal.
fn same_parse(x: &Expanded, d: Dialect) -> Result<(), String> {
    let by_lexer = rerail_c_lean::tape(&x.text, d).map(|t| rerail_c_lean::tape_text(&t));
    let by_tokens = rerail_c_lean::tape_of_tokens(&x.text, &x.toks, d).map(|t| rerail_c_lean::tape_text(&t));
    match (&by_lexer, &by_tokens) {
        _ if by_lexer == by_tokens => Ok(()),
        (Ok(a), Ok(b)) => {
            let k = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(0);
            let at = |s: &str| s.lines().nth(k).unwrap_or("").to_string();
            Err(format!("line {k}: lexer {:?} tokens {:?}", at(a), at(b)))
        }
        _ => Err(format!("lexer {:?} tokens {:?}", by_lexer.map(|s| s.len()), by_tokens.map(|s| s.len()))),
    }
}

#[test]
fn an_expanded_unit_parses_from_its_tokens() {
    let src = b"#include \"h.h\"\n#define MAX(a, b) ((a) > (b) ? (a) : (b))\n#define CAT(a, b) a ## b\nint CAT(x, 1) = MAX(1, 2);\nint f(void) <% return x1<:0:>; %>\n".to_vec();
    let x = expand(&[("main.c", src), ("h.h", b"typedef int T;\n".to_vec())], "main.c", Options::c(17, true));
    assert_eq!(String::from_utf8_lossy(&x.text), "typedef int T;\nint x1 = ((1) > (2) ? (1) : (2));\nint f(void) { return x1[0]; }");
    same_parse(&x, Dialect::C).unwrap();
    assert!(rerail_c_lean::tape_of_tokens(&x.text, &x.toks, Dialect::C).is_ok());
    // Origins: `typedef` is h.h's first byte; `((1)` is MAX's expansion; `int f` is main.c's line 5.
    let file_of = |at: usize| x.origin(at as u32).map(|(o, off)| (o.file, o.exp, off));
    assert_eq!(file_of(0), Some((2, 0, 0)));
    let max = x.text.windows(4).position(|w| w == b"((1)").unwrap();
    assert!(file_of(max).unwrap().1 != 0);
    let f = x.text.windows(5).position(|w| w == b"int f").unwrap();
    let main = b"#include \"h.h\"\n#define MAX(a, b) ((a) > (b) ? (a) : (b))\n#define CAT(a, b) a ## b\nint CAT(x, 1) = MAX(1, 2);\n";
    assert_eq!(file_of(f), Some((1, 0, main.len() as u32)));
    // A line's tokens that follow their file byte for byte are one segment.
    assert!(x.origins.len() < x.toks.len() / 2, "{} segments, {} tokens", x.origins.len(), x.toks.len());
}

#[test]
fn cpp_suffixes_are_names() {
    let x = expand(&[("main.cc", b"auto s = \"a\"_s + 'c'_c;\n".to_vec())], "main.cc", Options::cpp(20, true));
    let kinds: Vec<T> = x.toks.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, [T::Name, T::Name, T::Eq, T::Str, T::Name, T::Plus, T::Char, T::Name, T::Semi, T::Eof]);
    same_parse(&x, Dialect::Cpp).unwrap();
}

/// Every file of the corpora (their own text; headers not found are left out), parsed from its
/// tokens and from its text: the same tapes.
#[test]
fn the_corpora_parse_from_their_tokens_as_from_their_text() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let root = std::path::Path::new(&home).join("orca/rerail-shared/lean-corpus");
    let (mut files, mut parsed, mut differ) = (0, 0, Vec::new());
    for (dir, opts, d) in [("c", Options::c(17, true), Dialect::C), ("cpp", Options::cpp(20, true), Dialect::Cpp)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        for e in rd.flatten() {
            let path = e.path().to_string_lossy().into_owned();
            let Ok(src) = std::fs::read(&path) else { continue };
            let x = expand(&[(path.as_str(), src)], &path, opts);
            files += 1;
            parsed += usize::from(rerail_c_lean::tape_of_tokens(&x.text, &x.toks, d).is_ok());
            if let Err(m) = same_parse(&x, d) {
                differ.push(format!("{path}: {m}"));
            }
        }
    }
    eprintln!("{files} files, {parsed} parsed from their tokens, {} differ", differ.len());
    assert!(differ.is_empty(), "{}", differ[..differ.len().min(10)].join("\n"));
}
