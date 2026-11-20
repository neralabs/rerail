use rerail_c_pp::tape::{IDENTIFIER, image, image_16};
use rerail_metal::pp_eval::{Unit, evaluate, name_key, placement};
use rerail_metal::slabs;
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
/// A unit of `files` (each a source and whether it is C++), as the evaluator reads it: each
/// file's format 16 tape (`image_16`) and its source placed in the slab residency, where the
/// device reads them (decision 0460 §1). The bytes written into each room stand in for the
/// store's load of a stored tape (0353 step 4a).
fn place(files: &[(&[u8], bool)]) -> Unit {
    use sha2::{Digest as _, Sha256};
    let tapes: Vec<_> = files.iter().map(|&(s, cpp)| image_16(s, crate::pp::options(cpp))).collect();
    let digests: Vec<[u8; 32]> = files.iter().zip(&tapes).map(|(&(s, _), t)| Sha256::new().chain_update(t.as_bytes()).chain_update(s).finalize().into()).collect();
    let misses = slabs::misses(&digests).unwrap();
    let mut parts = Vec::new();
    for (k, d) in digests.iter().enumerate() {
        if !misses.contains(d) || parts.iter().any(|p: &slabs::Placement<'_>| p.0 == *d) {
            continue;
        }
        let (tape, src) = (tapes[k].as_bytes(), files[k].0);
        parts.push(placement(tape.len(), src.len(), *d, rerail_c_pp::tape::lang(), move |t: &mut [u8], s: &mut [u8]| {
            t.copy_from_slice(tape);
            s.copy_from_slice(src);
        }));
    }
    slabs::insert(&digests, parts).unwrap();
    Unit::of(slabs::resident(&digests).unwrap()).unwrap()
}

/// `corpus`'s files, each with whether it is C++.
fn corpus_in() -> Vec<(Vec<u8>, Vec<u8>, bool)> {
    let root = corpus_root();
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

/// R2 of decision 0460: the evaluator reads its files where the residency placed them. Its source
/// copies no tape and no source into a buffer of its own (0448's `Unit::load` did), reads no tape
/// through a reader of its own (the kernels' reads are the CLIF reads), and keeps no line index of
/// its own (R3: `__LINE__` is the tape's newline index's).
#[test]
fn the_evaluator_copies_no_tape() {
    let rust = include_str!("../../../metal/src/pp_eval.rs");
    let ukl = [include_str!("../../../metal/src/pp_eval.uglykl"), include_str!("../../../metal/src/pp_expand.uglykl")].concat();
    for line in rust.lines().filter(|l| !l.trim_start().starts_with("//")) {
        let copies = ["copy_nonoverlapping", "copy_from_slice", "extend_from_slice", "MappedBuffer::from_slice(&device, &data", "fn load("];
        if copies.iter().any(|c| line.contains(c)) {
            assert!(line.contains("copy: not block data"), "pp_eval.rs copies: {line}");
        }
    }
    for gone in ["HEADER_BYTES", "rd32(", "px_newlines", "px_line_blocks", "LINE_BLOCK"] {
        assert!(!ukl.contains(gone), "the evaluator still has `{gone}`");
    }
    assert!(ukl.contains("clif_tape_start") && ukl.contains("clif_tape_line"), "the evaluator reads through the CLIF reads");
}

/// §5 of decision 0460: the evaluator is UKL, launched through the lane: its source has no MSL
/// (no Metal kernel, attribute, address space or SIMD function of its own), takes UKL's work queue
/// and grid, and the preprocessor's MSL work queue is gone.
#[test]
fn the_evaluator_is_ukl() {
    let src = rerail_metal::pp_arena::source();
    for msl in ["kernel void", "[[buffer", "metal_stdlib", "device ", "threadgroup ", "simd_", "atomic_uint", "memory_order", "thread_position"] {
        let code: Vec<&str> = src.lines().filter(|l| !l.trim_start().starts_with("//") && l.contains(msl)).take(3).collect();
        assert!(code.is_empty(), "the evaluator's source has MSL `{msl}`: {code:?}");
    }
    assert!(src.contains("#include <ukl/workqueue.h>") && src.contains("#include <ukl/grid.h>"), "UKL's queue and grid");
    assert!(src.contains("DEFINE_KERNEL2(pp, pp_setup") && src.contains("DEFINE_KERNEL2(pp, px_expand") && src.contains("DEFINE_KERNEL2(pp, pp_check"), "the grid's kernels are UKL's");
    let arena = include_str!("../../../metal/src/pp_arena.rs");
    assert!(arena.contains("lane::Command") && !arena.contains("new_compute_command_encoder"), "launched through the lane");
}

/// R4 of decision 0460: an evaluation's stores are at offsets in one block, its arena
/// (`pp_arena::Arena`), leased from the GPU pool within the one budget: neither the front passes
/// nor the expansion makes a buffer of its own.
#[test]
fn the_arena_is_one_block() {
    for (name, rust) in [("pp_eval.rs", include_str!("../../../metal/src/pp_eval.rs")), ("pp_expand.rs", include_str!("../../../metal/src/pp_expand.rs"))] {
        assert!(!rust.contains("MappedBuffer"), "{name} makes a buffer of its own");
    }
    let blocks = include_str!("../../../metal/src/pp_arena.rs");
    assert!(blocks.contains("lease_block_within("), "the arena is leased within the budget");
}

/// The device's reader gives every node of every file as `View` does, and each identifier leaf's
/// key is its bytes' (`name_key`), every other node's 0; the unit's node bases are the files'
/// counts' prefix sum.
#[test]
fn the_names_pass_reads_the_tapes_in_place() {
    let mut files = vec![
        (b"#define F(a) a + x\nint y = F(abcdefghijklmnopq);\n".to_vec(), Vec::new(), true),
        (b"".to_vec(), Vec::new(), true),
        (b"x\n".to_vec(), Vec::new(), true),
    ];
    for f in &mut files {
        f.1 = image(&f.0, crate::pp::options(true));
    }
    files.extend(corpus_in());
    let unit = place(&files.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>());
    let got = evaluate(&unit, true).unwrap();
    let mut g = 0usize;
    for (f, (src, tape, _)) in files.iter().enumerate() {
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
    let unit = place(&files.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>());
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
    let unit = place(&chunk.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>());
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
    let unit = place(&p1.files.iter().map(|s| (s.as_slice(), cpp)).collect::<Vec<_>>());
    let mut lay = Layout { insts: Vec::new(), dir_slots: Vec::new(), dir_active: Vec::new(), gap_slots: Vec::new(), gap_active: Vec::new(), builtins: p1.builtins.iter().map(|n| name_key(n)).collect(), relex_options: {
        let o = crate::pp::options(cpp);
        rerail_metal::pp_expand::relex_options(o.cpp, o.colons, o.digit_separators, o.raw_strings)
    }, file_literals: {
        // Each file's `__FILE__` literal, then `__DATE__`'s, `__TIME__`'s and `__TIMESTAMP__`'s: the
        // string, its `"` and `\` escaped (the serial pass's `string_tok`).
        let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
        p1.paths.iter().chain([&cfg.date, &cfg.time, &cfg.timestamp]).map(|p| {
        let mut q = vec![b'"'];
        for &c in p.as_bytes() {
            if c == b'"' || c == b'\\' {
                q.push(b'\\');
            }
            q.push(c);
        }
        q.push(b'"');
        q
    }).collect() }, unit_floors: vec![0], unit_gaps: vec![0] };
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
    use rerail_metal::pp_expand::{L_BUILTIN, L_DATE, L_FILE, L_IDENT, L_LINE, L_NONE, L_NOT_NAME, L_TIME, L_TIMESTAMP, lookups};
    let p1 = p1_of(cfg, io, path)?;
    let (unit, lay, _) = unit_of(&p1, cpp);
    let (ev, got) = lookups(&unit, &lay).unwrap();
    same(got.len(), p1.lookups.len(), format!("{what}: the active text's tokens"));
    for (k, (&g, want)) in got.iter().zip(&p1.lookups).enumerate() {
        let g = match g {
            L_NOT_NAME => Lookup::NotName,
            L_NONE => Lookup::None,
            L_BUILTIN | L_LINE | L_FILE | L_DATE | L_TIME | L_TIMESTAMP | L_IDENT => Lookup::Builtin,
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
    let root = corpus_root();
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
            if g & rerail_metal::pp_expand::SCRATCH != 0 {
                let [a, b, _] = x.stoks[(g & !rerail_metal::pp_expand::SCRATCH) as usize];
                return x.scratch[a as usize..b as usize].to_vec();
            }
            let f = x.ev.bases.partition_point(|&b| b <= g) - 1;
            let i = (g - x.ev.bases[f]) as usize;
            // The serial pass spells a token after phase 2: its line splices left out.
            rerail_metal::pp_eval::phase2(&p1.files[f][views[f].start(i) as usize..views[f].end(i) as usize]).into_owned()
        })
        .collect();
    (Ok(spelled), serial.0, x.rung, x.launches)
}

/// Decision 0460 §4: an evaluation is a fixed handful of launches, `pp_setup`, `px_expand` twice
/// and `pp_check`, whatever the unit's size: an empty unit, one of thousands of tokens and
/// definitions, and one that runs again at a larger rung. A grid past the groups the device holds
/// at once fails its barrier, never hangs, and runs again at half the groups to the same output.
#[test]
fn an_evaluation_is_a_fixed_handful_of_launches() {
    let mut big = String::new();
    for k in 0..400usize {
        big += &format!("#define M{k}(a, b) (a + b * {k}) M{}\n", k.saturating_sub(1));
    }
    for k in 0..400 {
        big += &format!("int v{k} = M{k}(x{k}, y) + M{}(1, 2);\n", (k * 7) % 400);
    }
    let doubling = "#define A0 x y\n#define A1 A0 A0\n#define A2 A1 A1\n#define A3 A2 A2\n#define A4 A3 A3\n#define A5 A4 A4\n#define A6 A5 A5\n#define A7 A6 A6\n#define A8 A7 A7\n#define A9 A8 A8\nA9 A9 A9\n";
    let shapes: [(&str, &[(&str, &str)]); 3] = [("empty", &[("main.c", "")]), ("large", &[("main.c", big.as_str())]), ("doubling", &[("main.c", doubling)])];
    for (what, files) in shapes {
        let (got, want, rung, launches) = expanded(files, false, 0);
        same(got.map(|g| show(&g)), Ok(show(&want)), what);
        same(launches, 4, format!("{what}'s launches (rung {rung})"));
    }
    // 4,096 groups of 256 threads: past what any GPU holds at once.
    use rerail_c_pp::files::Search;
    let m: std::collections::HashMap<String, Vec<u8>> = [("main.c".to_string(), big.into_bytes())].into_iter().collect();
    let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
    let p1 = rerail_c_pp::stream::parallel::p1_of(rerail_c_pp::Config::new(crate::pp::options(false)), &mut io, "main.c").unwrap();
    let (unit, lay, _) = unit_of(&p1, false);
    let wide = rerail_metal::pp_expand::expand_at(&unit, &lay, 0, 4096).unwrap();
    let narrow = rerail_metal::pp_expand::expand_from(&unit, &lay, 0).unwrap();
    assert!(wide.groups < 4096, "the grid past residency ran at {} groups", wide.groups);
    same(wide.refused, 0, "the halved grid's refusals");
    same(&wide.out, &narrow.out, "the halved grid's output");
    eprintln!("past residency: ran again at {} groups", wide.groups);
}

/// A part's output spelled: each ref's token in its file (`files` and `tapes`, the unit's), each
/// scratch token's bytes.
fn part_tokens(part: &rerail_metal::pp_expand::Part, files: &[Vec<u8>], tapes: &[Vec<u8>]) -> Vec<Vec<u8>> {
    use rerail_metal::pp_expand::SCRATCH;
    let x = &*part.x;
    let views: Vec<View> = tapes.iter().map(|t| View::new(t).unwrap()).collect();
    x.out[part.out.clone()]
        .iter()
        .map(|&g| {
            if g & SCRATCH != 0 {
                let [a, b, _] = x.stoks[(g & !SCRATCH) as usize];
                return x.scratch[a as usize..b as usize].to_vec();
            }
            let f = x.ev.bases.partition_point(|&b| b <= g) - 1;
            let i = (g - x.ev.bases[f]) as usize;
            let f = f - part.files.start;
            // The serial pass spells a token after phase 2: its line splices left out.
            rerail_metal::pp_eval::phase2(&files[f][views[f].start(i) as usize..views[f].end(i) as usize]).into_owned()
        })
        .collect()
}

/// Decision 0460, batching: units expanded in one batch give each its own serial answer. The same
/// names are defined differently, undefined, or left undefined from unit to unit, so a unit that
/// saw an earlier unit's row would differ; `__FILE__` used after another unit redefined it; one
/// unit the device refuses (a call across a stretch's end), so the batch splits and that unit
/// alone is refused.
#[test]
fn a_batch_is_its_units() {
    use rerail_c_pp::files::Search;
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define A 1\n#define F(x) x A\nA F(2) B\n")],
        &[("main.c", "A F(3) B\n#define B b\nB\n")],
        &[("main.c", "#define A 2\n#undef F\nF(4) A\n")],
        &[("main.c", "#define __FILE__ x\n__FILE__ __LINE__\n")],
        &[("main.c", "__FILE__ __LINE__ A\n")],
        &[("main.c", "#define F(x) x\nF(\n#define A\n)\n")],
        &[("main.c", "#include \"a.h\"\nH K\n"), ("a.h", "#define K k\n#define H(x) x\n")],
        &[("main.c", "K H(1)\n")],
    ];
    let mut entries = Vec::new();
    for files in shapes {
        let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
        let cfg = rerail_c_pp::Config::new(crate::pp::options(false));
        let mut serial = Spelled::default();
        let m2 = m.clone();
        let mut io = Search::new(vec![], move |p: &str| m2.get(p).cloned());
        let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
        pp.run("main.c");
        drop(pp);
        let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
        let p1 = rerail_c_pp::stream::parallel::p1_of(cfg, &mut io, "main.c").unwrap();
        let (unit, lay, tapes) = unit_of(&p1, false);
        entries.push((serial.0, p1, unit, lay, tapes));
    }
    let units: Vec<_> = entries.iter().map(|e| (&e.2, &e.3)).collect();
    let parts = rerail_metal::pp_expand::expand_many(&units).unwrap();
    let mut alone = Vec::new();
    for (k, ((serial, p1, _, _, tapes), part)) in entries.iter().zip(&parts).enumerate() {
        if part.x.refused != 0 {
            alone.push((k, part.x.refused));
            continue;
        }
        same(show(&part_tokens(part, &p1.files, tapes)), show(serial), format!("unit {k}"));
    }
    eprintln!("refused alone: {alone:?}");
    same(alone, vec![(5, rerail_metal::pp_expand::X_ACROSS)], "the units refused: a call across a directive's stretch end");
    // The units before a refused one ran in a batch of more than one.
    assert!(std::rc::Rc::ptr_eq(&parts[0].x, &parts[1].x), "units 0 and 1 in one batch");
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

/// The shapes of `shapes`, each in C and C++, expanded on the device, against the serial pass.
fn shapes_agree(shapes: &[&[(&str, &str)]]) -> (usize, usize) {
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
    (tokens, launches)
}

/// R6 of decision 0448, its first part: calls with arguments, each parameter used expanded getting
/// its argument expanded once, in isolation (a fenced list, its paint kept when substituted), then
/// the body rescanned. Nested calls, calls in arguments and in bodies, parentheses and commas in
/// arguments, empty arguments, the wrong count of arguments (the name alone), variadic macros (the
/// variadic part left out, empty, or many), a name painted in an argument, and a function-like
/// name at an argument's end that its substitution calls.
#[test]
fn calls_with_arguments_are_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define F(x) x x\nF(1) F((a,b)) F() F( )\n")],
        &[("main.c", "#define G(a,b) b a\nG(1,2) G(G(1,2),3) G((1,2),(3))\n")],
        &[("main.c", "#define I(x) x\n#define A I(A)\nA I(A) I(I(I(1)))\n")],
        &[("main.c", "#define f(x) x(1)\n#define g(y) y+1\nf(g) f(f)\n")],
        &[("main.c", "#define F(x) x\nF(1,2) z F z F\n")],
        &[("main.c", "#define V(a, ...) a __VA_ARGS__\nV(1) V(1,2,3) V(1,) V((1,2),3)\n")],
        &[("main.c", "#define W(...) [__VA_ARGS__]\nW() W(a,b) W(W(1),W(2,3))\n")],
        &[("main.c", "#define H(x) [x]\n#define K H(H(1))\nK H(K) H(H(K))\n")],
        &[("main.c", "#define P(x,y) x|y\n#define Q P(1,P(2,3))\nQ P(Q,Q)\n")],
        &[("main.c", "#define N(x) x N\n#define M(x) N(x)\nM(1) M(M(2))\n")],
        &[("main.c", "#define E(x)\n#define Z(x, y) y\nE(1) Z(,) Z(a,) Z(,b)\n")],
        &[("main.c", "#define L(x) x\nL(\n1\n,\n) L(L\n(2))\n")],
        &[("main.c", "#define C(a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t,u,v,w,x,y,z,A,B,C,D,E,F,G,H) H G F E D C B A z y x w v u t s r q p o n m l k j i h g f e d c b a\nC(1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34)\n")],
        &[("main.c", "#include \"m.h\"\nT(T(1)) U(2)\n"), ("m.h", "#define T(x) (x)\n#define U(x) T(x) T(U(x))\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R6 of decision 0448, its second part: `#`, a parameter's raw argument as one string literal:
/// its tokens' spellings, a space where a token had space or a line's start before it (the first
/// token's left out), a literal's `"` and `\` escaped; an empty argument's `""`; spacing kept through
/// a body (a parameter's space, the call's name's space), and the string rescanned in its list.
#[test]
fn stringizing_is_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define S(x) #x\nS(a) S(  a  +  b  ) S() S(\"s\\n\" 'c') S(a\n b)\n")],
        &[("main.c", "#define S(x) #x\n#define T(x) S(x)\n#define A 1 + 2\nS(A) T(A) T( A ) T(S(A))\n")],
        &[("main.c", "#define Q(x, y) # x # y x\nQ(1,2) Q((a,b),c) Q(,)\n")],
        &[("main.c", "#define V(...) #__VA_ARGS__\nV() V(a,b , c) V( a )\n")],
        &[("main.c", "#define S(x) #x\nS(a/**/b) S(a/**/ b) S('\\'' \"\\\\\")\n")],
        &[("main.c", "#define S(x) #x\n#define F(x) [x]\nF(S(1)) F( S( 1 ) )\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R6 of decision 0448, its third part: `##`, chains of it left to right, a paste that spells no
/// one token (both kept, the right one starting the next), placemarkers (an empty argument beside
/// it), pastes that make names (rescanned), numbers, punctuators and digraphs, `#` beside `##`,
/// and GNU's `, ## __VA_ARGS__` (the comma gone with no variadic arguments, kept with some).
#[test]
fn pasting_is_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define C(a, b) a ## b\nC(x, y) C(1, 2) C(+, =) C(<, :) C(, b) C(a, ) C(,) C(x y, z w)\n")],
        &[("main.c", "#define C3(a, b, c) a ## b ## c\nC3(1, 2, 3) C3(, , c) C3(a, , ) C3(+, +, +) C3(x, ., y)\n")],
        &[("main.c", "#define C(a, b) a ## b\n#define xy done\n#define AB C(x, y)\nC(x, y) AB C(C, (x, y))\n")],
        &[("main.c", "#define H(a) #a ## x\n#define K(a) a ## #a\nH(1) K(y)\n")],
        &[("main.c", "#define E(fmt, ...) f(fmt, ## __VA_ARGS__)\nE(1) E(1, 2) E(1, 2, 3) E(1,)\n")],
        &[("main.c", "#define G(...) g(0, ## __VA_ARGS__)\nG() G(a) G(a, b)\n")],
        &[("main.c", "#define P(x) x ## _t\n#define Q(x) P(x)\n#define N 9\nP(N) Q(N) P(int)\n")],
        &[("main.c", "#define L(x) x ## L\n#define U(x) u ## x\nL(1) L(0x1) U(8) U(\"s\")\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R6 of decision 0448, its fourth part: `__VA_OPT__` (C++20, C23): its content where the variadic
/// argument, expanded, has tokens (an argument that expands to nothing has none), else a
/// placemarker (so `##` beside it pastes with nothing); its content's parameters and pastes, and
/// its space given to its content's first token.
#[test]
fn va_opt_is_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define F(a, ...) f(a __VA_OPT__(,) __VA_ARGS__)\nF(1) F(1, 2) F(1, 2, 3) F(1,)\n")],
        &[("main.c", "#define E\n#define G(...) [__VA_OPT__(x __VA_ARGS__ y)]\nG() G(E) G(a) G( a , b )\n")],
        &[("main.c", "#define H(x, ...) x ## __VA_OPT__(_v)\n#define K(x, ...) __VA_OPT__(x ## x)\nH(a) H(a, 1) K(b) K(b, 1)\n")],
        &[("main.c", "#define S(...) __VA_OPT__((__VA_ARGS__)) end\nS() S(()) S(a, (b))\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// The lean corpus: `RERAIL_PP_CORPUS` (a copy sent with a run to a farm host), else the shared one.
fn corpus_root() -> std::path::PathBuf {
    std::env::var("RERAIL_PP_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"))
}

/// A batch's GPU time by stage and by phase, as `where_an_evaluations_gpu_time_goes` gives a
/// unit's: each the best of 5, at 32 groups.
fn batch_stages(units: &[(&rerail_metal::pp_eval::Unit, &rerail_metal::pp_expand::Layout)]) {
    use rerail_metal::pp_eval::{TO_ALL, TO_DEFS, TO_FRONT, TO_VIS, Unit};
    use rerail_metal::pp_expand::{Layout, stage_seconds};
    let Some(unit) = Unit::batch(&units.iter().map(|(u, _)| *u).collect::<Vec<_>>()) else { return eprintln!("no batch: slabs differ") };
    let joined: Vec<(&Layout, usize)> = units.iter().map(|(u, l)| (*l, u.len())).collect();
    let lay = Layout::join(&joined).unwrap();
    let best = |f: &dyn Fn() -> f64| (0..5).map(|_| f()).fold(f64::MAX, f64::min) * 1e6;
    let t: Vec<String> = [("front", TO_FRONT), ("defs", TO_DEFS), ("vis", TO_VIS), ("setup", TO_ALL + 1), ("all", TO_ALL)]
        .iter()
        .map(|&(n, last)| format!("{n} {:.0}", best(&|| stage_seconds(&unit, &lay, last, 0, 32).unwrap().unwrap_or(f64::NAN))))
        .collect();
    eprintln!("a batch of {} units (us, cumulative): {}", units.len(), t.join(", "));
    let at: Vec<f64> = (1..=60).map(|k| best(&|| stage_seconds(&unit, &lay, TO_ALL + 1, k, 32).unwrap().unwrap_or(f64::NAN))).collect();
    let steps: Vec<String> = at.windows(2).enumerate().map(|(k, w)| format!("{}:{:.0}", k + 1, w[1] - w[0])).collect();
    eprintln!("  stopped before phase 1: {:.0} us; phase k's cost: {}", at[0], steps.join(" "));
}

/// R10 of decision 0448: the corpus expanded on the device, each file its own unit with empty
/// stubs for its headers, against the serial pass; every unit the device does not refuse gives
/// the serial pass's tokens, and the refusals are counted by reason. `RERAIL_PP_FIRST` and
/// `RERAIL_PP_COUNT` choose the files of each language; `RERAIL_PP_STAGES` measures a batch.
#[test]
#[ignore = "the corpus (~/orca/rerail-shared/lean-corpus); run in slices"]
fn the_corpus_expands_on_the_device() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    use rerail_metal::pp_expand::*;
    let root = corpus_root();
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
    let first = std::env::var("RERAIL_PP_FIRST").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let reasons = [
        (X_BUILTIN, "built-ins"),
        (X_PAST_END, "a call past a list's end"),
        (X_PASTE, "## in a row"),
        (X_DEPTH, "depth"),
        (X_ROOM, "room at the last rung"),
        (X_QUEUE, "the queue"),
        (X_HASH, "#"),
        (X_ACROSS, "a call across a stretch's end"),
        (X_VA_OPT, "#__VA_OPT__"),
        (X_RELEX, "a paste the relexer refuses"),
        (X_SPLICE, "# or ## of a token with a line splice"),
        (X_BARRIER, "a grid barrier"),
    ];
    let (mut same_units, mut tokens, mut launches) = (0, 0, 0);
    let mut gpu: Vec<f64> = Vec::new();
    let mut batches: Vec<usize> = Vec::new();
    let mut refused = std::collections::BTreeMap::<&str, usize>::new();
    let mut named = std::collections::BTreeMap::<String, usize>::new();
    let stub = |q: &str| std::fs::read(q).ok().or_else(|| q.starts_with("<stub>").then(Vec::new));
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        // Each unit's serial answer and its P1, then every unit of the language in batches.
        struct Entry {
            path: String,
            serial: Vec<Vec<u8>>,
            p1: rerail_c_pp::stream::parallel::P1,
            unit: rerail_metal::pp_eval::Unit,
            lay: Layout,
            tapes: Vec<Vec<u8>>,
        }
        let mut entries = Vec::new();
        for p in paths.into_iter().skip(first).take(count) {
            let path = p.to_string_lossy().into_owned();
            let cfg = Config::new(crate::pp::options(cpp));
            let mut serial = Spelled::default();
            let mut io = Search::new(vec![("<stub>".into(), DirKind::After)], stub);
            let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
            pp.run(&path);
            drop(pp);
            let mut io = Search::new(vec![("<stub>".into(), DirKind::After)], stub);
            let p1 = match rerail_c_pp::stream::parallel::p1_of(cfg, &mut io, &path) {
                Ok(p1) => p1,
                Err(why) => {
                    *refused.entry(why).or_default() += 1;
                    continue;
                }
            };
            let (unit, lay, tapes) = unit_of(&p1, cpp);
            entries.push(Entry { path, serial: serial.0, p1, unit, lay, tapes });
        }
        let units: Vec<_> = entries.iter().map(|e| (&e.unit, &e.lay)).collect();
        // `RERAIL_PP_STAGES`: where a batch's GPU time goes (its first `RERAIL_PP_STAGES` units).
        if let Some(k) = std::env::var("RERAIL_PP_STAGES").ok().and_then(|k| k.parse::<usize>().ok()) {
            batch_stages(&units[..k.min(units.len())]);
        }
        let parts = match expand_many(&units) {
            Ok(parts) => parts,
            Err(e) => panic!("{dir}: {e:?}"),
        };
        let mut seen: Vec<*const Expanded> = Vec::new();
        for (e, part) in entries.iter().zip(&parts) {
            let (path, p1, x) = (&e.path, &e.p1, &*part.x);
            if !seen.contains(&std::ptr::from_ref(x)) {
                seen.push(std::ptr::from_ref(x));
                launches = launches.max(x.launches);
                gpu.push(x.gpu_seconds);
                batches.push(x.unit_out.len().saturating_sub(1).max(1));
            }
            // Room counts only alone: with another refusal the unit is not rerun, and the other is why.
            let refused_by = if x.refused & !X_ROOM != 0 { x.refused & !X_ROOM } else { x.refused };
            if refused_by & X_ROOM != 0 {
                let full: Vec<_> = ROOM_STORES.iter().zip(x.room).filter(|(_, (u, c))| u > c).map(|(n, (u, c))| format!("{n} {u}/{c}")).collect();
                eprintln!("ROOM {path} at rung {}: {}", x.rung, full.join(", "));
            }
            // The built-ins a unit refused for them names in its text.
            if refused_by & X_BUILTIN != 0 {
                for name in &p1.builtins {
                    let n = name.as_slice();
                    if p1.files.iter().any(|f| f.windows(n.len()).any(|w| w == n)) {
                        *named.entry(String::from_utf8_lossy(n).into_owned()).or_default() += 1;
                    }
                }
            }
            if x.refused != 0 {
                for (bit, why) in reasons {
                    if refused_by & bit != 0 {
                        *refused.entry(why).or_default() += 1;
                    }
                }
                continue;
            }
            let got = part_tokens(part, &p1.files, &e.tapes);
            if got != e.serial {
                let k = got.iter().zip(&e.serial).position(|(a, b)| a != b).unwrap_or(got.len().min(e.serial.len()));
                let near = |v: &[Vec<u8>]| show(&v[k.saturating_sub(8)..(k + 8).min(v.len())]);
                eprintln!("DIFF {path}: token {k} (device {}, serial {})\n  device: {}\n  serial: {}", got.len(), e.serial.len(), near(&got), near(&e.serial));
                panic!("{path}");
            }
            same_units += 1;
            tokens += got.len();
        }
    }
    eprintln!("{same_units} units the serial pass's ({tokens} tokens), at most {launches} launches; refused {refused:?}");
    gpu.sort_by(f64::total_cmp);
    if let Some(&most) = gpu.last() {
        let all: f64 = gpu.iter().sum();
        eprintln!("GPU time: {:.1} ms in all over {} runs (units a run: {batches:?}), median {:.3} ms, most {:.3} ms a run", all * 1e3, gpu.len(), gpu[gpu.len() / 2] * 1e3, most * 1e3);
    }
    eprintln!("the built-ins in units refused for them: {named:?}");
}

/// R7 of decision 0448: calls whose `(` or `)` is past their list's end. The call reads the
/// frames after it raw (R-peek), its list is made in the frame that holds its `)` (R-ctx), and what
/// it reads is rolled back: a sibling call it covers is dead (its tokens read raw into the
/// arguments), and so is that sibling's own exit, even one reaching past the live call's end.
/// Chains of such calls, exits through two lists, exits inside a macro's list and inside an
/// argument's, the fence an argument's end makes, C17 §6.10.3.5's example 3, and DEFER/OBSTRUCT.
#[test]
fn calls_past_an_end_are_the_serial_passes() {
    let example3 = "#define x 3\n#define f(a) f(x * (a))\n#undef x\n#define x 2\n#define g f\n#define z z[0]\n#define h g(~\n#define m(a) a(w)\n#define w 0,1\n#define t(a) a\n#define p() int\n#define q(x) x\n#define r(x,y) x ## y\n#define str(x) # x\nf(y+1) + f(f(z)) % t(t(g)(0) + t)(1);\ng(x+(3,4)-w) | h 5) & m\n(f)^m(m);\np() i[q()] = { q(1), r(2,3), r(4,), r(,5), r(,) };\nchar c[2][6] = { str(hello), str() };\n";
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define f(x) g\n#define g(y) y + 1\n#define F f\nf(1)(2) F(3)(4)\n")],
        &[("main.c", "#define h(a) [a]\n#define H h(\nH 5) end\n")],
        &[("main.c", "#define f(x) g\n#define g(y) y\nf(1) + 2\n")],
        &[("main.c", example3)],
        &[("main.c", "#define f(x) g\n#define g(y) [y]\n#define h(z) <z>\nf(1)(h(2)) end h(3)\n")],
        &[("main.c", "#define f(a) g\n#define g(a) #a\n#define y k((\n#define k(a) {a}\nf(0)( y ) z ) )\n")],
        &[("main.c", "#define f(a) g\n#define g(a) [a]\n#define y k\n#define k(a) {a}\nf(0)( y (1) ) z\n")],
        &[("main.c", "#define f(x) g\n#define g(x) f\nf(1)(2)(3)(4) end\n")],
        &[("main.c", "#define A B\n#define B C\n#define C(x) <x>\nA(1) x A (2)\n")],
        &[("main.c", "#define H h(\n#define HH H\n#define h(a) [a]\nHH 5) end HH (6, 7)) x\n")],
        &[("main.c", "#define f(x) g\n#define g(y) [y]\n#define W(z) f(1)(z) w\nW(2)\n")],
        &[("main.c", "#define f(x) g\n#define g(y) [y]\n#define h(z) <z>\n#define W(z) f(1)(h(z)) w\nW(2) W(h(3))\n")],
        &[("main.c", "#define f(x) g\n#define g(y) [y]\n#define t(a) a\nt(f(1))(2) t(f(1)(3))\n")],
        &[("main.c", "#define EMPTY()\n#define DEFER(id) id EMPTY()\n#define OBSTRUCT(...) __VA_ARGS__ DEFER(EMPTY)()\n#define EXPAND(...) __VA_ARGS__\n#define A() 123\nDEFER(A)() EXPAND(DEFER(A)()) OBSTRUCT(A)() EXPAND(OBSTRUCT(A)()) EXPAND(EXPAND(OBSTRUCT(A)()))\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R8 of decision 0448: a unit past its rung's room. An 8⁴ fan-out used three times wants more
/// than rung 0's arena of tokens, and a `#` of 512 long names more scratch bytes than rung 0's
/// 64 KB: the device refuses (`X_ROOM`), the unit runs again at the next rung, and it gives the
/// serial pass's tokens there.
#[test]
fn a_unit_past_its_room_runs_again_larger() {
    let fan = "#define A0 x x x x x x x x\n#define A1 A0 A0 A0 A0 A0 A0 A0 A0\n#define A2 A1 A1 A1 A1 A1 A1 A1 A1\n#define A3 A2 A2 A2 A2 A2 A2 A2 A2\n";
    let tokens = format!("{fan}A3 A3 A3\n");
    // 512 names of 256 bytes: 131 KB stringized.
    let name = "n".repeat(256);
    let bytes = format!("#define B0 {name} {name} {name} {name} {name} {name} {name} {name}\n#define B1 B0 B0 B0 B0 B0 B0 B0 B0\n#define B2 B1 B1 B1 B1 B1 B1 B1 B1\n#define S(a) #a\n#define X(a) S(a)\nX(B2) end\n");
    for (what, text) in [("tokens", tokens), ("scratch bytes", bytes)] {
        let files = [("main.c", text.as_str())];
        for cpp in [false, true] {
            let (got, want, rung, _) = expanded(&files, cpp, 0);
            let got = got.unwrap_or_else(|why| panic!("{what}: refused {why}"));
            same(show(&got), show(&want), what);
            assert!(rung > 0, "{what}: ran at rung 0");
            eprintln!("{what}: {} tokens at rung {rung}", got.len());
        }
    }
}

/// Plan step 11 of decision 0448 (the built-ins): `__LINE__` is the line of the outermost call's
/// end (the serial pass's: a token's origin climbed through each expansion's end), `__FILE__` the
/// file the stretch is in. In text, in a body, in an argument (expanded and raw, `#`), in a call
/// over many lines, in an exit's call, and in an included file; `__DATE__`, `__TIME__` and
/// `__TIMESTAMP__` (the configuration's), and `__has_include` and its kin as plain names in text.
#[test]
fn line_and_file_are_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "__LINE__ __FILE__\n\n  __LINE__\n#define L __LINE__\nL\n#define F __FILE__\nF\n")],
        &[("main.c", "#define call(a, b) a b\ncall(__LINE__,\n  __LINE__) call(L,\n\n L)\n#define L __LINE__\ncall(L,\n L)\n")],
        &[("main.c", "#define S(a) #a\n#define X(a) S(a)\n#define I(a) a\nS(__LINE__) X(__LINE__) I(\nI(\n__LINE__))\n")],
        &[("main.c", "#define f(x) g\n#define g(y) __LINE__ y\n#define G g\nf(1)\n(2)\nG\n(\n3)\n")],
        &[("main.c", "#include \"i.h\"\n__FILE__ __LINE__ H\n"), ("i.h", "\n#define H __FILE__ __LINE__\n__FILE__ __LINE__ H\n")],
        &[("main.c", "// a line\n/* two\nlines */ __LINE__ \\\n __LINE__\n#define C(a, b) a ## b\nC(__LI, NE__) C(__FILE, __)\n")],
        &[("main.c", "__DATE__ __TIME__ __TIMESTAMP__\n#define D __DATE__ x\nD __has_include __has_include(<a.h>) __has_embed(\"e\") __has_include_next\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// A measurement, not a check: each stage's GPU time for a small and a large unit, at 1 and 32
/// groups, and each phase's of `pp_setup` (an empty dispatch's: `what_a_barrier_costs`).
#[test]
#[ignore = "a measurement"]
fn where_an_evaluations_gpu_time_goes() {
    use rerail_c_pp::files::Search;
    use rerail_metal::pp_eval::{TO_ALL, TO_DEFS, TO_FRONT, TO_VIS};
    use rerail_metal::pp_expand::stage_seconds;
    let mut big = String::new();
    for k in 0..400usize {
        big += &format!("#define M{k}(a, b) (a + b * {k}) M{}\n", k.saturating_sub(1));
        big += &format!("int v{k} = M{k}(x{k}, y) + M{}(1, 2);\n", (k * 7) % 400);
    }
    let best = |f: &dyn Fn() -> f64| (0..5).map(|_| f()).fold(f64::MAX, f64::min) * 1e6;
    for (what, src) in [("small", "#define A 1\nint x = A;\n".to_string()), ("large", big)] {
        let m: std::collections::HashMap<String, Vec<u8>> = [("main.c".to_string(), src.into_bytes())].into_iter().collect();
        let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
        let p1 = rerail_c_pp::stream::parallel::p1_of(rerail_c_pp::Config::new(crate::pp::options(false)), &mut io, "main.c").unwrap();
        let (unit, lay, _) = unit_of(&p1, false);
        for g in [1u64, 32] {
            let t: Vec<String> = [("front", TO_FRONT), ("defs", TO_DEFS), ("vis", TO_VIS), ("setup", TO_ALL + 1), ("all", TO_ALL)]
                .iter()
                .map(|&(n, last)| format!("{n} {:.0}", best(&|| stage_seconds(&unit, &lay, last, 0, g).unwrap().unwrap_or(f64::NAN))))
                .collect();
            eprintln!("{what}, {g} groups (us, cumulative): {}", t.join(", "));
            // Each phase's cost: the run stopped before phase k+1 less the run stopped before k.
            let at: Vec<f64> = (1..=60).map(|k| best(&|| stage_seconds(&unit, &lay, TO_ALL + 1, k, g).unwrap().unwrap_or(f64::NAN))).collect();
            let steps: Vec<String> = at.windows(2).enumerate().map(|(k, w)| format!("{}:{:.0}", k + 1, w[1] - w[0])).collect();
            eprintln!("  stopped before phase 1: {:.0} us; phase k's cost: {}", at[0], steps.join(" "));
        }
    }
}
