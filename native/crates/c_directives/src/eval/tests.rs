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

/// A unit read by the spike's P1, on the device: its files with their tapes, and its layout as
/// test input (0448: until P1's includes and conditionals run on the device).
fn unit_of(p1: &rerail_c_pp::stream::parallel::P1, cpp: bool) -> (Unit, rerail_metal::pp_expand::Layout, Vec<Vec<u8>>) {
    use rerail_metal::pp_expand::{Layout, PpInst};
    let tapes: Vec<Vec<u8>> = p1.files.iter().map(|s| image(s, crate::pp::options(cpp))).collect();
    let unit = Unit::load(&p1.files.iter().zip(&tapes).map(|(s, t)| (s.as_slice(), t.as_slice())).collect::<Vec<_>>()).unwrap();
    let mut lay = Layout { insts: Vec::new(), dir_slots: Vec::new(), dir_active: Vec::new(), gap_slots: Vec::new(), gap_active: Vec::new(), builtins: p1.builtins.iter().map(|n| name_key(n)).collect() };
    for i in &p1.insts {
        lay.insts.push(PpInst { file: i.file, dir_first: lay.dir_slots.len() as u32, gap_first: lay.gap_slots.len() as u32 });
        let slot = |s: u64| u32::try_from(s).expect("a slot in 32 bits");
        lay.dir_slots.extend(i.dir_slots.iter().map(|&s| slot(s)));
        lay.dir_active.extend(i.dir_active.iter().map(|&a| u32::from(a)));
        lay.gap_slots.extend(i.gap_slots.iter().map(|&s| slot(s)));
        lay.gap_active.extend(i.gap_active.iter().map(|&a| u32::from(a)));
    }
    (unit, lay, tapes)
}

/// One unit of R4: its main file read by the spike's P1, then its layout's lookups on the device
/// against `Vis::at`'s. `None` where P1 refuses it (the reason counted).
fn lookups_agree(cfg: rerail_c_pp::Config, io: &mut dyn rerail_c_pp::files::Files, path: &str, cpp: bool, what: &str) -> Result<[usize; 3], &'static str> {
    use rerail_c_pp::stream::parallel::{Lookup, p1_of};
    use rerail_metal::pp_expand::{L_BUILTIN, L_NONE, L_NOT_NAME, lookups};
    let p1 = p1_of(cfg, io, path)?;
    let (unit, lay, _) = unit_of(&p1, cpp);
    let (ev, got) = lookups(&unit, &lay).unwrap();
    same(got.len(), p1.lookups.len(), format!("{what}: the active text's tokens"));
    for (k, (&g, want)) in got.iter().zip(&p1.lookups).enumerate() {
        let g = match g {
            L_NOT_NAME => Lookup::NotName,
            L_NONE => Lookup::None,
            L_BUILTIN => Lookup::Builtin,
            r => {
                let f = ev.dir_bases.partition_point(|&b| b <= r) - 1;
                Lookup::Def { file: f as u32, dir: r - ev.dir_bases[f] }
            }
        };
        same(g, *want, format!("{what}: token {k}"));
    }
    let defs = p1.lookups.iter().filter(|l| matches!(l, Lookup::Def { .. })).count();
    let builtins = p1.lookups.iter().filter(|l| matches!(l, Lookup::Builtin)).count();
    Ok([got.len(), defs, builtins])
}

/// R4 of decision 0448: every token of the unit's active text has the meaning on the device that
/// the spike's `Vis::at` gives it (no name, no macro, a built-in, or the `#define` it sees),
/// from the visibility rows (a row an active `#define` or `#undef`, one stable sort) and a
/// search a token. On shapes of includes, conditionals, redefinitions and undefs, then on the
/// corpus, each file its own unit with empty stubs for the headers it names.
#[test]
fn every_name_sees_the_spikes_definition() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define A 1\nA B\n#undef A\nA\n#define A 2\nA __LINE__ __FILE__\n")],
        &[("main.c", "#include \"a.h\"\nX Y\n#include \"a.h\"\nX\n"), ("a.h", "#ifndef X\n#define X x\n#else\n#undef X\n#define Y y\n#endif\n")],
        &[("main.c", "#if defined(Q)\n#define R 1\n#elif 1\n#define R 2\n#endif\nR\n#define Q\n#include \"b.h\"\nR\n"), ("b.h", "#ifdef Q\n#define R 3\nR\n#endif\n")],
        &[("main.c", "#define F(x) x\nF(F)(1)\n#define F 0\nF\n#include \"c.h\"\nF\n"), ("c.h", "F\n#undef F\nF\n")],
        &[("main.c", "int main(void) { return 0; }\n")],
        &[("main.c", "")],
    ];
    let mut tokens = [0; 3];
    let mut add = |n: [usize; 3]| (0..3).for_each(|i| tokens[i] += n[i]);
    for (k, files) in shapes.iter().enumerate() {
        for cpp in [false, true] {
            let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
            let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
            add(lookups_agree(Config::new(crate::pp::options(cpp)), &mut io, "main.c", cpp, &format!("shape {k}")).unwrap());
        }
    }
    let root = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus");
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
    let first = std::env::var("RERAIL_PP_FIRST").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let (mut units, mut refused) = (0, std::collections::BTreeMap::<&str, usize>::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths.into_iter().skip(first).take(count) {
            let path = p.to_string_lossy().into_owned();
            // Every header a stub, empty: `<>` ones after the search path, `""` ones beside it.
            let mut io = Search::new(vec![("<stub>".into(), DirKind::After)], |q: &str| std::fs::read(q).ok().or_else(|| q.starts_with("<stub>").then(Vec::new)));
            match lookups_agree(Config::new(crate::pp::options(cpp)), &mut io, &path, cpp, &path) {
                Ok(n) => {
                    units += 1;
                    add(n);
                }
                Err(why) => *refused.entry(why).or_default() += 1,
            }
        }
    }
    eprintln!("{} shapes, {units} corpus units: {} tokens, {} of a definition, {} of a built-in; refused {refused:?}", shapes.len() * 2, tokens[0], tokens[1], tokens[2]);
}

/// The serial pass's output tokens, spelled.
#[derive(Default)]
struct Spelled(Vec<Vec<u8>>);

impl rerail_c_pp::sink::Sink for Spelled {
    fn token(&mut self, _t: &rerail_c_pp::macros::Tk, text: &[u8], _at: Option<rerail_c_pp::sink::At>, _off: u32) {
        self.0.push(text.to_vec());
    }
}

/// The unit of `files` (its main file `main.c`) expanded on the device from rung `rung`: its
/// output spelled, or its refusal bits; and the serial pass's output. `None` where P1 refuses it.
fn expanded(files: &[(&str, &str)], cpp: bool, rung: u32) -> (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, u32, usize) {
    use rerail_c_pp::files::Search;
    let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
    let mut serial = Spelled::default();
    let m2 = m.clone();
    let mut io = Search::new(vec![], move |p: &str| m2.get(p).cloned());
    let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
    let p1 = rerail_c_pp::stream::parallel::p1_of(cfg, &mut io, "main.c").unwrap();
    let (unit, lay, tapes) = unit_of(&p1, cpp);
    let x = rerail_metal::pp_expand::expand_from(&unit, &lay, rung).unwrap();
    if x.refused != 0 {
        return (Err(x.refused), serial.0, x.rung, x.launches);
    }
    let views: Vec<View> = tapes.iter().map(|t| View::new(t).unwrap()).collect();
    let spelled = x
        .out
        .iter()
        .map(|&g| {
            let f = x.ev.bases.partition_point(|&b| b <= g) - 1;
            let i = (g - x.ev.bases[f]) as usize;
            p1.files[f][views[f].start(i) as usize..views[f].end(i) as usize].to_vec()
        })
        .collect();
    (Ok(spelled), serial.0, x.rung, x.launches)
}

fn show(v: &[Vec<u8>]) -> String {
    v.iter().map(|t| String::from_utf8_lossy(t).into_owned()).collect::<Vec<_>>().join(" ")
}

/// R5 of decision 0448: object-like calls and calls without arguments, expanded on the device
/// (the top level's grid passes, then the queue's EXPAND and PLACE items), give the serial
/// pass's tokens: self and mutual recursion, painting through a chain, empty bodies, a
/// function-like name with no `(` after it (in its list, or after its list's end in a parent),
/// redefinitions between stretches, names from an included header, and a doubling chain.
#[test]
fn object_like_expansion_is_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define A 1\nA B A\n")],
        &[("main.c", "#define A A x\nA\n")],
        &[("main.c", "#define A B\n#define B A\nA B\n")],
        &[("main.c", "#define A B C\n#define B C 1\n#define C A\nA C B\n")],
        &[("main.c", "#define X Y\n#define Y Z\n#define Z X Y Z\nX Y Z\n")],
        &[("main.c", "#define F() f F\nF() F ( ) F\n")],
        &[("main.c", "#define G() G()\nG()\n")],
        &[("main.c", "#define F() x\nF y F\n")],
        &[("main.c", "#define F() f\n#define A F\nA x\n#define B A\nB y\n")],
        &[("main.c", "#define F() 1\n#define A F() F()\nA\n")],
        &[("main.c", "#define E\nE x E E\nE\n")],
        &[("main.c", "#define A 1\nA\n#undef A\nA\n#define A 2\nA\n")],
        &[("main.c", "#include \"a.h\"\nH x K\n#define K L\nK\n"), ("a.h", "#define K k H\n#define H(x) x\nK\n")],
        &[("main.c", "#define A0 x\n#define A1 A0 A0\n#define A2 A1 A1\n#define A3 A2 A2\n#define A4 A3 A3\n#define A5 A4 A4\n#define A6 A5 A5\nA6 A6\n")],
        &[("main.c", "int main(void) { return 0; }\n")],
        &[("main.c", "")],
    ];
    let (mut tokens, mut launches) = (0, 0);
    for (k, files) in shapes.iter().enumerate() {
        for cpp in [false, true] {
            let (got, want, _, n) = expanded(files, cpp, 0);
            launches = launches.max(n);
            let at = format!("shape {k} ({})", if cpp { "C++" } else { "C" });
            match got {
                Ok(got) => {
                    same(show(&got), show(&want), &at);
                    tokens += got.len();
                }
                Err(why) => same(why, 0, format!("{at}: refused")),
            }
        }
    }
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}
