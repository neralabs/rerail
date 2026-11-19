use rerail_c_pp::tape::{IDENTIFIER, image};
use rerail_metal::pp_eval::{Unit, evaluate, name_key};
use rerail_tape_format::View;

/// `a == b`, or the difference printed and a panic (Relax keeps no assert's message).
#[track_caller]
fn same<T: PartialEq + std::fmt::Debug>(a: T, b: T, what: impl std::fmt::Display) {
    if a != b {
        eprintln!("MISMATCH {what}: {a:?} for {b:?}");
        panic!("{what}");
    }
}

/// Files of the corpora (`RERAIL_PP_COUNT` of each language, 40 by default), each with its tape.
fn corpus() -> Vec<(Vec<u8>, Vec<u8>)> {
    corpus_in().into_iter().map(|(s, t, _)| (s, t)).collect()
}

/// `corpus`'s files, each with whether it is C++.
fn corpus_in() -> Vec<(Vec<u8>, Vec<u8>, bool)> {
    let root = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus");
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
    let mut out = Vec::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths.into_iter().take(count) {
            let src = std::fs::read(&p).unwrap();
            let tape = image(&src, crate::pp::options(cpp));
            out.push((src, tape, cpp));
        }
    }
    out
}

/// The device's reader gives every node of every file as `View` does, and each identifier leaf's
/// key is its bytes' (`name_key`), every other node's 0; the unit's node bases are the files'
/// counts' prefix sum.
#[test]
fn the_names_pass_reads_the_tapes_in_place() {
    let mut files = vec![
        (b"#define F(a) a + x\nint y = F(abcdefghijklmnopq);\n".to_vec(), Vec::new()),
        (b"".to_vec(), Vec::new()),
        (b"x\n".to_vec(), Vec::new()),
    ];
    for f in &mut files {
        f.1 = image(&f.0, crate::pp::options(true));
    }
    files.extend(corpus());
    let unit = Unit::load(&files.iter().map(|(s, t)| (s.as_slice(), t.as_slice())).collect::<Vec<_>>()).unwrap();
    let got = evaluate(&unit, true).unwrap();
    let mut g = 0usize;
    for (f, (src, tape)) in files.iter().enumerate() {
        let v = View::new(tape).unwrap();
        same(got.bases[f] as usize, g, format!("file {f}'s first node"));
        for i in 0..v.len() {
            let want = [v.key(i), v.start(i), v.end(i), v.size(i) as u32];
            same(got.nodes[g], want, format!("file {f} node {i}"));
            let key = if (v.key(i) & 0x3ff) as u16 == IDENTIFIER { name_key(&src[v.start(i) as usize..v.end(i) as usize]) } else { 0 };
            same(got.keys[g], key, format!("file {f} node {i}'s key"));
            g += 1;
        }
    }
    same(got.total as usize, g, "the total");
    eprintln!("{} files, {g} nodes", files.len());
}

/// The directives pass (a file pass, a prefix sum, a file pass) gives each file's directives as
/// the spike's P0 tables do (`parallel::tables`): kind, text tokens before, the group each is
/// read in and the one after it, its chain; and each file's groups. Each row's node is a
/// directive of its kind in the tape, and its name key (`pp_dir_keys`) is the key of the name
/// the spike's directive defines, undefines or tests. Each file's guard key is the spike's
/// guard's name's.
#[test]
fn the_directives_pass_gives_the_spikes_tables() {
    let shapes: [&[u8]; 13] = [
        b"#define F(a) a\nint y = F(1);\n",
        b"#ifndef G\n#define G\n#if A\nx y\n#elif B\n#if C\nz\n#endif\n#else\nw\n#endif\n#endif\n",
        b"#else\n#endif\n#elif X\nt\n#if A\n#else\n#else\nu\n",
        b"",
        b"a\n#\n#pragma once\n#include <x.h>\nb c d\n",
        b"#if A\n#if B\n#endif\n",
        b"#ifndef G\n#define G 1\n#if A\n#else\n#endif\n#endif\n",
        b"#ifndef G\n#define G 1\n#endif\nx\n",
        b"#ifndef G\n#else\n#endif\n",
        b"#ifndef G\n#endif\n#define H\n",
        b"x\n#ifndef G\n#endif\n",
        b"#ifndef 1\n#endif\n",
        b"#define\n#define 1\n#define G(x) x\n#undef\n#undef G H\n#ifdef\n#elifdef X\n#endif\n#ifndef abcdefghijklmnopqrs\n#elifndef Y\n#endif\n#elifdef Z\n",
    ];
    let mut files: Vec<(Vec<u8>, Vec<u8>, bool)> = shapes.iter().map(|s| (s.to_vec(), image(s, crate::pp::options(false)), false)).collect();
    files.extend(corpus_in());
    let unit = Unit::load(&files.iter().map(|(s, t, _)| (s.as_slice(), t.as_slice())).collect::<Vec<_>>()).unwrap();
    let got = evaluate(&unit, false).unwrap();
    let mut rows = 0usize;
    for (f, (src, tape, cpp)) in files.iter().enumerate() {
        let tables = rerail_c_pp::stream::parallel::tables(crate::pp::options(*cpp), src);
        let (want, groups) = (&tables.dirs, tables.groups);
        same(got.guards[f], tables.guard.as_deref().map_or(0, name_key), format!("file {f}'s guard {:?}", tables.guard.as_deref().map(String::from_utf8_lossy)));
        let v = View::new(tape).unwrap();
        same(got.dir_bases[f] as usize, rows, format!("file {f}'s first row"));
        same(got.groups[f], groups, format!("file {f}'s groups"));
        for (d, w) in want.iter().enumerate() {
            let r = got.rows[rows + d];
            same((r.kind, r.text_before, r.group, r.after, r.chain), (u32::from(w.kind), w.text_before, w.group, w.after, w.chain), format!("file {f} directive {d}"));
            same(v.key(r.node as usize) & 0x3ff, r.kind, format!("file {f} directive {d}'s node"));
            same(got.dir_keys[rows + d], w.name.as_deref().map_or(0, name_key), format!("file {f} directive {d}'s name {:?}", w.name.as_deref().map(String::from_utf8_lossy)));
        }
        rows += want.len();
    }
    same(got.rows.len(), rows, "the rows");
    same(got.dir_keys.len(), rows, "the rows' keys");
    eprintln!("{} files, {rows} directives, {} guarded", files.len(), got.guards.iter().filter(|&&g| g != 0).count());
}

/// R2 of decision 0448: every `#define`'s definition, as the setup makes it from the tape on the
/// device (`pp_expand::definitions`: S1 a thread a row, S2 a thread a parameter list's leaf, S4 a
/// thread a body leaf), is the serial pass's (`Pp::parse_define`, through the spike's
/// `parallel::tables`): malformed or not, function-like, variadic, the parameters' names, and each
/// body token's spelling, parameter, paste operator and white space before it.
#[test]
fn the_definitions_are_the_serial_passes() {
    use rerail_metal::pp_expand::{D_BAD, D_DEFINE, D_FNLIKE, D_VARIADIC, I_PASTE, I_SPACE, NO_PARAM, definitions};
    let shapes: [&[u8]; 13] = [
        b"#define A\n#define B 1\n#define C (1)\n#define D(x) x\n",
        b"#define F(a, b) a ## b # a b##a\n#define G(...) __VA_ARGS__\n#define H(a, ...) a __VA_ARGS__\n",
        b"#define I(a...) a\n#define J() j\n#define K(a,) k\n#define L(a b) l\n#define M(... x) m\n",
        b"#define N(a\n#define\n#define 1\n#define O(a, a) a\n#define P ## x ##\n",
        b"#define Q   spaced   out  \n#define R(x)x/**/y\n#define S a\\\n  b\n",
        b"#define T(x, y) #x #y x ## y __VA_OPT__(x)\n#undef T\n#define U(x) x\n",
        b"#define V(...) __VA_OPT__(a ## __VA_ARGS__)\n#define W(x...) #x\n",
        b"#if 1\n#define X 1\n#else\n#define X 2\n#endif\n",
        b"#define Y(\n#define Z(a,b,c,d,e,f,g,h,i,j) j i h g f e d c b a\n",
        b"#define AA (x) y\n#define BB() \n#define CC(a)\t a\n",
        b"#define DD <a.h>\n#define EE \"s\" 'c' L\"w\" 1.2e+3\n",
        b"#define FF(a) a##\n#define GG(a) ##a\n#define HH(a) a ## ## a\n",
        b"#define II std\\\nint \\  \n x\n#define JJ(a\\\n) a\n",
    ];
    let mut files: Vec<(Vec<u8>, Vec<u8>, bool)> = Vec::new();
    for s in shapes {
        for cpp in [false, true] {
            files.push((s.to_vec(), image(s, crate::pp::options(cpp)), cpp));
        }
    }
    files.extend(corpus_in());
    // Units of 500 files: a batch's buffers stay inside the device's budget (0448 §2).
    let (mut defines, mut malformed) = (0usize, 0usize);
    for (c, chunk) in files.chunks(500).enumerate() {
    let unit = Unit::load(&chunk.iter().map(|(s, t, _)| (s.as_slice(), t.as_slice())).collect::<Vec<_>>()).unwrap();
    let (got, d) = definitions(&unit).unwrap();
    let mut rows = 0usize;
    for (f, (src, tape, cpp)) in chunk.iter().enumerate() {
        let f = c * 500 + f;
        let tables = rerail_c_pp::stream::parallel::tables(crate::pp::options(*cpp), src);
        let v = View::new(tape).unwrap();
        let base = got.bases[f % 500] as usize;
        for (k, w) in tables.dirs.iter().enumerate() {
            let r = rows + k;
            let def = d.defs[r];
            let at = format!("file {f} directive {k}");
            if w.kind != rerail_c_pp::tape::kind_id(rerail_c_pp::directives::Kind::Define) {
                same(def.flags, 0, format!("{at}: no definition"));
                continue;
            }
            defines += 1;
            same(def.flags & D_DEFINE, D_DEFINE, format!("{at}: a define"));
            let Some(want) = &w.def else {
                malformed += 1;
                same(def.flags & D_BAD, D_BAD, format!("{at}: malformed"));
                continue;
            };
            same(def.flags & D_BAD, 0, format!("{at}: well-formed"));
            same((def.flags & D_FNLIKE != 0, def.flags & D_VARIADIC != 0), (want.fnlike, want.variadic), format!("{at}: function-like, variadic"));
            if want.fnlike {
                let first = def.pfirst as usize;
                let params: Vec<u64> = (1..def.n_params as usize).step_by(2).map(|k| d.pkeys[first + k]).filter(|&k| k != 0).collect();
                same(params, want.params.iter().map(|p| name_key(p)).collect(), format!("{at}: the parameters {:?}", want.params.iter().map(|p| String::from_utf8_lossy(p)).collect::<Vec<_>>()));
            }
            same(def.n_body as usize, want.body.len(), format!("{at}: the body's tokens"));
            for (j, (text, param, paste, space)) in want.body.iter().enumerate() {
                let i = def.body as usize - base + j;
                let spelled = rerail_metal::pp_eval::phase2(&src[v.start(i) as usize..v.end(i) as usize]);
                let item = d.items[def.ifirst as usize + j];
                let p = item & 0xffff;
                same(&*spelled, text.as_slice(), format!("{at}: body token {j}'s spelling"));
                same((if p == NO_PARAM { None } else { Some(p as usize) }, item & I_PASTE != 0, item & I_SPACE != 0), (*param, *paste, *space), format!("{at}: body token {j} {:?}", String::from_utf8_lossy(text)));
            }
        }
        rows += tables.dirs.len();
    }
    same(d.defs.len(), rows, "the rows");
    }
    eprintln!("{} files, {defines} defines, {malformed} malformed", files.len());
}

/// R3 of decision 0448: the paste relexer (`pp_expand.metal`'s `relex`) answers as the lean
/// lexer does (`Pp::paste`: one token from the first byte to the last, not `Other`), with the
/// token's tape kind, for every pair of a set of spellings (every punctuator, digraph, `#`, `##`,
/// names, literal prefixes, numbers, literals), under C17, C23, GNU C, C++17 and C++20.
#[test]
fn the_paste_relexer_is_the_lexers() {
    use rerail_c_lean::pp::{DIGRAPH, Options, P, PpLexer};
    use rerail_metal::pp_expand::{RELEX_NONE, relex, relex_options};
    let one: &[&str] = &[
        "(", ")", "[", "]", "{", "}", ";", ",", "~", "?", ":", "::", ".", "...", ".*", "=", "==", "!", "!=", "+", "++", "+=", "-", "--", "-=", "->", "->*", "*", "*=", "/",
        "/=", "%", "%=", "^", "^=", "&", "&&", "&=", "|", "||", "|=", "<", "<<", "<<=", "<=", "<=>", ">", ">>", ">>=", ">=", "<:", ":>", "<%", "%>", "%:", "%:%:", "#", "##",
        "a", "_x", "$", "L", "u", "U", "u8", "R", "LR", "x1", "e", "E", "p", "_", "1", "0x", "1e", "1.", ".5", "1'0", "'", "\"", "\"s\"", "'c'", "L\"w\"", "u8'c'", "\"s\"_x",
        "0", "9e", "+1", "'0", "1'", ">", "=>", ".", "..", "<::", "*/", "//", "\"(x)\"", "\"d(x)d\"", "(x)\"", "\"\\\"\"", "'\\n'",
    ];
    let mut texts = Vec::new();
    for a in one {
        texts.push(a.as_bytes().to_vec());
        for b in one {
            texts.push(format!("{a}{b}").into_bytes());
        }
    }
    let dialects = [("C17", Options::c(17, false)), ("C23", Options::c(23, false)), ("GNU C17", Options::c(17, true)), ("C++17", Options::cpp(17, false)), ("C++20", Options::cpp(20, true))];
    for (name, opts) in dialects {
        let got = relex(&texts, relex_options(opts.cpp, opts.colons, opts.digit_separators, opts.raw_strings)).unwrap();
        for (t, g) in texts.iter().zip(got) {
            let tk = PpLexer::new(t, opts).next();
            let whole = tk.start == 0 && tk.end as usize == t.len() && !matches!(tk.kind, P::Eof | P::Other | P::Header);
            let want = if !whole {
                RELEX_NONE
            } else {
                u32::from(match tk.kind {
                    P::Ident => rerail_c_pp::tape::IDENTIFIER,
                    P::Number => rerail_c_pp::tape::NUMBER,
                    P::Char | P::Str | P::RawStr => rerail_c_pp::tape::LITERAL,
                    P::Hash => rerail_c_pp::tape::HASH,
                    P::HashHash => rerail_c_pp::tape::HASHHASH,
                    P::Punct if tk.flags & DIGRAPH != 0 => rerail_c_pp::tape::DIGRAPH_KIND,
                    _ => rerail_c_pp::tape::PUNCT,
                })
            };
            same(g, want, format!("{name}: {:?}", String::from_utf8_lossy(t)));
        }
    }
    eprintln!("{} spellings, 5 dialects", texts.len());
}
