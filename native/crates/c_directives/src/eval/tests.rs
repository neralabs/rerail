use rerail_c_pp::tape::{IDENTIFIER, PUNCT, image};
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

use super::rounds::place;

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

/// R2 of decision 0468: the evaluator reads its files where the residency placed them. Its source
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

/// §5 of decision 0468: the evaluator is UKL, launched through the lane: its source has no MSL
/// (no Metal kernel, attribute, address space or SIMD function of its own) and takes UKL's grid.
/// Its expansion is a loop a thread a piece (decision 0588): no work queue, no rounds, no pool.
#[test]
fn the_evaluator_is_ukl() {
    let src = rerail_metal::pp_arena::source();
    for msl in ["kernel void", "[[buffer", "metal_stdlib", "device ", "threadgroup ", "simd_", "atomic_uint", "memory_order", "thread_position"] {
        let code: Vec<&str> = src.lines().filter(|l| !l.trim_start().starts_with("//") && l.contains(msl)).take(3).collect();
        assert!(code.is_empty(), "the evaluator's source has MSL `{msl}`: {code:?}");
    }
    assert!(src.contains("#include <ukl/grid.h>"), "UKL's grid");
    assert!(!src.contains("#include <ukl/workqueue.h>") && !src.contains("ukl_wq_"), "no work queue (decision 0588)");
    for rounds in ["px_drain", "PX_DRAIN", "px_item", "ukl_pool_", "<ukl/pool.h>", "PxList"] {
        assert!(!src.contains(rounds), "the rounds' `{rounds}` (decision 0588: the loop alone)");
    }
    assert!(src.contains("DEFINE_KERNEL2(pp, pp_setup") && !src.contains("DEFINE_KERNEL2(pp, px_expand") && src.contains("DEFINE_KERNEL2(pp, pp_check"), "the grid's kernels are UKL's");
    let arena = include_str!("../../../metal/src/pp_arena.rs");
    assert!(arena.contains("lane::Command") && !arena.contains("new_compute_command_encoder"), "launched through the lane");
}

/// R4 of decision 0468: an evaluation's stores are at offsets in one block, its arena
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
/// key is its bytes' (`name_key`), a one-byte punctuator's its byte under `0x100`, every other
/// node's 0; the unit's node bases are the files' counts' prefix sum.
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
    let unit = place(&files.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>()).unwrap();
    let got = evaluate(&unit, true).unwrap();
    let mut g = 0usize;
    for (f, (src, tape, _)) in files.iter().enumerate() {
        let v = View::new(tape).unwrap();
        same(got.bases[f] as usize, g, format!("file {f}'s first node"));
        for i in 0..v.len() {
            let want = [v.key(i), v.start(i), v.end(i), v.size(i) as u32];
            same(got.nodes[g], want, format!("file {f} node {i}"));
            let text = &src[v.start(i) as usize..v.end(i) as usize];
            let key = match (v.key(i) & 0x3ff) as u16 {
                IDENTIFIER => name_key(text),
                PUNCT if text.len() == 1 => 0x100 | u64::from(text[0]),
                _ => 0,
            };
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
    let unit = place(&files.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>()).unwrap();
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
/// body token's spelling, parameter, paste operator, white space before it, and whether it is `#`.
#[test]
fn the_definitions_are_the_serial_passes() {
    use rerail_metal::pp_expand::{D_BAD, D_DEFINE, D_FNLIKE, D_VARIADIC, I_HASH, I_PASTE, I_SPACE, NO_PARAM, definitions};
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
    let unit = place(&chunk.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>()).unwrap();
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
                same(item & I_HASH != 0, text.as_slice() == b"#" || text.as_slice() == b"%:", format!("{at}: body token {j} {:?} is `#`", String::from_utf8_lossy(text)));
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
            let tk = PpLexer::new(t, opts).next_token();
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

/// A unit on the device from its main file alone, in P1's rounds of lookups (decision 0467 §1):
/// the last round's unit and layout, and its files' paths.
struct OnDevice {
    unit: Unit,
    lay: rerail_metal::pp_expand::Layout,
    paths: Vec<String>,
}

/// The unit whose main file is `path` on the device, in rounds, and the spike's P1 of it (the
/// cross-check of the passes' arrays; `Err` with its reason where the spike refuses it); or why
/// the device refuses it.
fn unit_of(cfg: rerail_c_pp::Config, io: &mut dyn rerail_c_pp::files::Files, path: &str, cpp: bool) -> Result<(OnDevice, Result<rerail_c_pp::stream::parallel::P1, &'static str>), &'static str> {
    let p1 = rerail_c_pp::stream::parallel::p1_of(cfg.clone(), io, path);
    Ok((on_device(cfg, io, path, cpp)?, p1))
}

/// The unit whose main file is `path` on the device, in rounds, the spike not run; or why the
/// device refuses it.
fn on_device(cfg: rerail_c_pp::Config, io: &mut dyn rerail_c_pp::files::Files, path: &str, cpp: bool) -> Result<OnDevice, &'static str> {
    use rerail_metal::pp_expand::*;
    // The rooms carry from round to round (decision 0486 §6).
    let mut sizes: Option<Sizes> = None;
    let r = super::rounds::run(&cfg, io, path, cpp, |u, l| {
        let mut s = sizes.map_or_else(|| Sizes::first(u, l), |s| s.fit(u, l));
        let r = lookups_sized(u, l, &mut s).map(|(_, _, o)| (o.requests, o.flags, o.refused));
        sizes = Some(s);
        r
    })
    .map_err(|e| why_refused(e, path))?;
    if r.answer & X_BUILTIN != 0 {
        return Err("built-ins in an #if");
    }
    if r.answer & X_PRAGMA != 0 {
        return Err("a _Pragma that makes a row");
    }
    if r.answer != 0 {
        return Err("an #if's expansion");
    }
    Ok(OnDevice { unit: r.unit, lay: r.lay, paths: r.loaded.paths })
}

/// Why the rounds refused the unit whose main file is `path`.
fn why_refused(e: super::rounds::Refused, path: &str) -> &'static str {
    use super::rounds::Refused;
    use rerail_metal::pp_expand::*;
    match e {
        Refused::NoMain => "the main file is not read",
        Refused::P1(f) if f & P1_IF_DEFINED != 0 => "defined made by a macro in an #if",
        Refused::P1(f) if f & P1_IF_BIG != 0 => "an #if past the evaluator's room",
        Refused::P1(f) if f & P1_EMBED != 0 => "#embed",
        Refused::P1(f) if f & P1_ROOM != 0 => "P1's room",
        Refused::P1(_) => "P1's iterations",
        Refused::Rounds => "past the rounds' bound",
        Refused::Device(e) => {
            eprintln!("device error {path}: {e:?}");
            "a device error"
        }
    }
}

/// What a language's batched rounds ran: the device's runs, and their GPU time (seconds).
#[derive(Default)]
struct Ran {
    runs: usize,
    gpu_seconds: f64,
    /// Each expansion store's most use over a run's top-level tokens (`ROOM_STORES`), and the
    /// most over its room.
    per_top: [f64; rerail_metal::pp_expand::ROOMS],
    per_cap: [f64; rerail_metal::pp_expand::ROOMS],
}

/// A unit settled by the rounds (`settled_many`): its expansion and its files' tapes, or why the rounds refused it.
type SettledUnit = Result<(super::rounds::Settled, Vec<Vec<u8>>), &'static str>;

/// The units whose main files are `units`' (each its files), settled in batched rounds
/// (`rounds::run_many`) under `cfg`: each its expansion and its files' tapes, or why the rounds
/// refused it; the runs counted in `ran`.
fn settled_many(cfg: &rerail_c_pp::Config, units: &mut [(Box<dyn rerail_c_pp::files::Files + '_>, String)], ran: &mut Ran) -> Vec<SettledUnit> {
    let cpp = cfg.opts.cpp;
    let r = super::rounds::run_many(cfg, cpp, units, |pairs, sizes| {
        let t0 = std::time::Instant::now();
        let xs = rerail_metal::pp_expand::expand_teams(pairs, sizes);
        if let Ok(xs) = &xs {
            // The batch's GPU time is each answer's (`Expanded::run`): counted once.
            let gpu = xs.iter().flatten().map(|x| x.gpu_seconds).next().unwrap_or(0.0);
            if std::env::var_os("RERAIL_PP_TIME").is_some() {
                eprintln!("TIME expand {} units {:.3} ms (device {:.3} ms)", pairs.len(), t0.elapsed().as_secs_f64() * 1e3, gpu * 1e3);
            }
            ran.runs += 1;
            ran.gpu_seconds += gpu;
            for x in xs.iter().flatten() {
                for (k, &(u, c)) in x.room.iter().enumerate() {
                    ran.per_top[k] = ran.per_top[k].max(f64::from(u) / x.caps.top.max(1) as f64);
                    ran.per_cap[k] = ran.per_cap[k].max(f64::from(u) / f64::from(c.max(1)));
                }
            }
        }
        xs
    });
    r.into_iter()
        .zip(units.iter())
        .map(|(r, (_, path))| {
            let s = r.map_err(|e| why_refused(e, path))?;
            let tapes = s.loaded.files.iter().map(|f| image(f, crate::pp::options(cpp))).collect();
            Ok((s, tapes))
        })
        .collect()
}

/// One unit of R4: its main file read by the spike's P1, then its layout's lookups on the device
/// against `Vis::at`'s. `None` where P1 refuses it (the reason counted).
fn lookups_agree(cfg: rerail_c_pp::Config, io: &mut dyn rerail_c_pp::files::Files, path: &str, cpp: bool, what: &str) -> Result<[usize; 3], &'static str> {
    use rerail_c_pp::stream::parallel::Lookup;
    use rerail_metal::pp_expand::{L_BASE_FILE, L_BUILTIN, L_COUNTER, L_DATE, L_PRAGMA, L_FILE, L_FEATURE, L_FILE_NAME, L_IDENT, L_INCLUDE_LEVEL, L_LINE, L_NONE, L_NOT_NAME, L_TIME, L_TIMESTAMP, lookups};
    let (d, p1) = unit_of(cfg, io, path, cpp)?;
    let p1 = p1?;
    let (ev, got, out) = lookups(&d.unit, &d.lay).unwrap();
    same(out.flags, 0, format!("{what}: P1's refusals"));
    same(out.requests.len(), 0, format!("{what}: the last round's requests"));
    // R1, R3 and R5 of decision 0467: the tree the device grew, its layout and its activity are
    // the spike's: every live instance's directives and stretches, by slot, with their file's
    // path, their index in it, and whether they are active.
    let dirs_of = |f: usize| (if f + 1 < ev.dir_bases.len() { ev.dir_bases[f + 1] } else { ev.rows.len() as u32 }) - ev.dir_bases[f];
    let (mut dev_d, mut dev_g) = (Vec::new(), Vec::new());
    for (i, n) in out.nodes.iter().enumerate() {
        if out.live[i] == 0 {
            continue;
        }
        let (f, d0) = (n.file as usize, out.dir_first[i] as usize);
        let g0 = d0 + i;
        for k in 0..dirs_of(f) as usize {
            dev_d.push((out.dir_slots[d0 + k], d.paths[f].clone(), k, out.dir_active[d0 + k]));
        }
        for k in 0..=dirs_of(f) as usize {
            dev_g.push((out.gap_slots[g0 + k], d.paths[f].clone(), k, out.gap_active[g0 + k]));
        }
    }
    let (mut want_d, mut want_g) = (Vec::new(), Vec::new());
    for i in &p1.insts {
        let f = i.file as usize;
        for k in 0..i.dir_slots.len() {
            want_d.push((i.dir_slots[k] as u32, p1.paths[f].clone(), k, u32::from(i.dir_active[k])));
        }
        for k in 0..i.gap_slots.len() {
            want_g.push((i.gap_slots[k] as u32, p1.paths[f].clone(), k, u32::from(i.gap_active[k])));
        }
    }
    for v in [&mut dev_d, &mut dev_g, &mut want_d, &mut want_g] {
        v.sort();
    }
    same(dev_d, want_d, format!("{what}: the directives (slot, file, index, active)"));
    same(dev_g, want_g, format!("{what}: the stretches (slot, file, index, active)"));
    same(got.len(), p1.lookups.len(), format!("{what}: the active text's tokens"));
    for (k, (&g, want)) in got.iter().zip(&p1.lookups).enumerate() {
        let g = match g {
            L_NOT_NAME => Lookup::NotName,
            L_NONE => Lookup::None,
            L_BUILTIN | L_LINE | L_FILE | L_DATE | L_TIME | L_TIMESTAMP | L_FILE_NAME | L_BASE_FILE | L_INCLUDE_LEVEL | L_COUNTER | L_FEATURE | L_PRAGMA | L_IDENT => Lookup::Builtin,
            r => {
                let f = ev.dir_bases.partition_point(|&b| b <= r) - 1;
                // By path: the device's files are in its rounds' order, the spike's in its own.
                let p = p1.paths.iter().position(|p| *p == d.paths[f]).expect("a file the spike read") as u32;
                Lookup::Def { file: p, dir: r - ev.dir_bases[f] }
            }
        };
        same(g, *want, format!("{what}: token {k}"));
    }
    let defs = p1.lookups.iter().filter(|l| matches!(l, Lookup::Def { .. })).count();
    let builtins = p1.lookups.iter().filter(|l| matches!(l, Lookup::Builtin)).count();
    Ok([got.len(), defs, builtins])
}

/// R4 of decision 0467: every `#if` and `#elif` is decided on the device as the serial pass's
/// `if_value` decides it. Each expression below chooses its own text (`t` or `f` and its index),
/// and the unit's output is the serial pass's, in C and C++. The expressions cover each operator
/// and its precedence, the unsigned rules, the short circuits (a division by zero not evaluated
/// is no error), the errors (which make a test false), character constants, names (`true` as the
/// dialect has it), macros expanded in the line, `defined` in both forms before and after a
/// definition, `__has_include`, `__has_include_next` and `__has_embed` (a header found, not found,
/// empty), the configuration's `__has_*` answers, and C++'s alternative tokens; and chains of
/// `#elif`, nested conditionals, and a test that a header's definition decides.
#[test]
fn if_values_are_the_serial_passes() {
    use rerail_c_pp::files::DirKind;
    let exprs = [
        "1 + 2 * 3 == 7", "(1 + 2) * 3 == 9", "10 - 2 - 3 == 5", "1 << 4 >> 2 == 4", "(7 & 3 | 8 ^ 1) == 11", "1 ? 2 : 3", "0 ? 2 : 0 ? 3 : 4",
        "0 ? 1 : 0", "-5 / 2 == -2", "-5 % 2 == -1", "!0 + ~0", "1, 0", "0, 1", "1 ? 0, 1 : 0", "0x10 + 010 + 0b11 == 27", "07 == 7", "0 == 00",
        "1 < 2 && 2 <= 2 && 3 > 2 && 3 >= 3 && 1 != 2", "-1 > 0u", "-1 > 0", "(0u - 1) >> 63", "-1 >> 63 == -1", "-1 >> 64 == -1", "1 << 64",
        "18446744073709551615 == -1", "(1 ? -1 : 0u) > 0", "(0 ? 0u : -1) > 0", "0xFFFFFFFFFFFFFFFFull", "99999999999999999999", "1u << 63 >> 63 == 1",
        "-9223372036854775807 - 1 < 0", "(-9223372036854775807 - 1) / -1 < 0", "(-9223372036854775807 - 1) % -1 == 0", "10LL == 10", "10uLL > 9",
        "0 && 1 / 0", "1 || 1 % 0", "0 ? 1 / 0 : 5", "1 / 0", "1 && 1 / 0", "!(0 && 1 / 0)", "(1 || 1 / 0) + 0", "1 % 0 || 1",
        "", "1 +", "1 2", "(1", "1.0", "1 )", "1 ? 2", ": 1", "\"s\"", "1 = 1", "1e5", "0x", "08", "1 ++ 2", "+ - ~ ! 0",
        "UNDEFINED + 1", "UNDEFINED", "true", "false", "true + true == 2",
        "'a' == 97", "'\\n' + '\\x41' + '\\101' == 140", "'\\377' < 0", "'ab' == 0x6162", "L'\\xffffffff' == -1", "U'\\xffffffff' == 0xffffffff",
        "u'\u{e9}' == 0xe9", "u8'a' == 97", "'\\u00e9' < 0", "'\\e' == 27", "''", "'\\q' == 'q'",
        "A == 2", "defined A", "defined(A)", "defined ( B )", "!defined C", "defined A && A == 2", "defined(F) && F(3) == 3", "defined LATER",
        "F(A) * 2 == 4", "G == 7", "H(1, 2) == 3", "F(F(A)) == 2", "__LINE__ > 0", "defined __LINE__", "defined", "defined (",
        "__has_include(\"a.h\")", "__has_include(\"nope.h\")", "__has_include(<b.h>)", "__has_include(<nope.h>)", "__has_include(<sub/c.h>)",
        "__has_include_next(<b.h>)", "__has_embed(\"a.h\") == 1", "__has_embed(\"e.h\") == 2", "__has_embed(\"nope.h\") == 0",
        "__has_feature(cxx_rtti)", "__has_feature(nope)", "__has_builtin(__builtin_expect)", "__has_cpp_attribute(nodiscard) >= 201603",
        "__has_cpp_attribute(clang::fallthrough)", "__is_identifier(foo)", "__has_feature", "defined __has_feature", "__has_attribute(F)",
        "1 and 1", "0 or 1", "not 0", "(6 bitand 3) == 2", "compl 0 == -1", "1 not_eq 2", "(5 xor 1) == 4", "(4 bitor 1) == 5",
    ];
    let mut main = String::from("#define A 2\n#define B\n#define F(x) x\n#define G (3 + 4)\n#define H(a, b) a + b\n");
    for (k, e) in exprs.iter().enumerate() {
        main += &format!("#if {e}\nt{k}\n#else\nf{k}\n#endif\n");
        if k == exprs.len() / 2 {
            main += "#define LATER\n#if defined LATER\nlater\n#endif\n#undef A\n#if defined A\nstill\n#endif\n#define A 2\n";
        }
    }
    main += "#if 0\na\n#elif 1 + 1 == 2\nb\n#elif 1\nc\n#else\nd\n#endif\n";
    main += "#if 1\n# if 0\nx\n# elif defined F\ny\n# endif\n#elif 1 / 0\nz\n#endif\n";
    main += "#ifdef A\n#if A > 1 && !defined NOPE\nnested\n#endif\n#endif\n";
    main += "#include \"a.h\"\n#if FROM_A == 5\nfrom_a\n#else\nnot_from_a\n#endif\n";
    let files = [("main.c", main.as_str()), ("a.h", "#define FROM_A 5\n"), ("e.h", ""), ("inc/b.h", "int b;\n"), ("inc/sub/c.h", "int c;\n")];
    for cpp in [false, true] {
        let mut cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
        for (t, op, v) in [("__has_feature", "cxx_rtti", 1), ("__has_builtin", "__builtin_expect", 1), ("__has_cpp_attribute", "nodiscard", 201603), ("__has_cpp_attribute", "clang::fallthrough", 201603), ("__has_attribute", "F", 1)] {
            cfg.features.insert((t.to_string(), op.to_string()), v);
        }
        let (got, want, rung, launches) = expanded_in(&files, cfg, &[("inc", DirKind::Angle)]);
        let chosen: Vec<String> = want.iter().map(|t| String::from_utf8_lossy(t).into_owned()).filter(|t| (t.starts_with('t') || t.starts_with('f')) && t.len() > 1 && t[1..].bytes().all(|c| c.is_ascii_digit())).collect();
        // Every test chose its text, some true and some false.
        same(chosen.len(), exprs.len(), "the tests' texts");
        assert!(chosen.iter().any(|t| t.starts_with('t')) && chosen.iter().any(|t| t.starts_with('f')), "both values");
        same(got.map(|g| show(&g)), Ok(show(&want)), format!("the tests' text, C++ {cpp}"));
        same(launches, LAUNCHES, format!("launches (rung {rung})"));
    }
}

/// R5 of decision 0467 (literal names): the device grows the include tree from the main file
/// alone, in rounds, and it is the spike's (checked inside `lookups_agree`: every live
/// instance's directives and stretches by slot, with their file, and every name's meaning).
/// Quoted names found by the includer's directory and by the search, `<…>` names, a quoted name
/// that resolves differently from two includers, `#include_next`, guards, `#pragma once`,
/// `#import`, a header entered twice with no guard, an include under a conditional a later
/// definition decides, a file not found, and the depth's bound.
#[test]
fn the_include_tree_is_the_spikes() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#include \"a.h\"\n#include <b.h>\nA B\n"), ("a.h", "#define A a\n"), ("inc/b.h", "#define B b\n")],
        &[("main.c", "#include \"sub/x.h\"\n#include \"y.h\"\nY\n"), ("sub/x.h", "#include \"y.h\"\n"), ("sub/y.h", "#define Y sub\n"), ("y.h", "#undef Y\n#define Y top\n")],
        &[("main.c", "#include <n.h>\nN\n"), ("inc/n.h", "#define N 1\n#include_next <n.h>\n"), ("sys/n.h", "#undef N\n#define N 2\n")],
        &[("main.c", "#include \"g.h\"\n#include \"g.h\"\n#include \"o.h\"\n#include \"o.h\"\nG O\n"), ("g.h", "#ifndef G_H\n#define G_H\n#define G g\nint g;\n#endif\n"), ("o.h", "#pragma once\nint o;\n#define O o\n")],
        &[("main.c", "#import \"i.h\"\n#include \"i.h\"\n#include \"t.h\"\n#include \"t.h\"\nI\n"), ("i.h", "int i;\n#define I i\n"), ("t.h", "twice\n")],
        &[("main.c", "#ifdef LATER\n#include \"no.h\"\n#else\n#include \"yes.h\"\n#endif\n#define LATER\n#ifdef LATER\n#include \"later.h\"\n#endif\n#include \"missing.h\"\nY L\n"), ("yes.h", "#define Y y\n"), ("later.h", "#define L l\n")],
        &[("main.c", "#include \"r.h\"\nR\n"), ("r.h", "#ifndef R\n#define R 0\n#else\n#undef R\n#define R deeper\n#endif\n#include \"r.h\"\n")],
    ];
    for (k, files) in shapes.iter().enumerate() {
        for cpp in [false, true] {
            let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
            let mut io = Search::new(vec![("inc".into(), DirKind::Angle), ("sys".into(), DirKind::System)], move |p: &str| m.get(p).cloned());
            let mut cfg = Config::new(crate::pp::options(cpp));
            // The last shape includes itself: the depth's bound ends it.
            cfg.max_depth = 6;
            lookups_agree(cfg, &mut io, "main.c", cpp, &format!("shape {k}")).unwrap();
        }
    }
}

/// R5 of decision 0467 (expanded names, step 6): an include whose name is a macro's expansion,
/// and a header test whose operand is one, resolve on the device as `Pp::header_of` spells the
/// name: a string, an object-like or function-like macro giving `<…>` (its tokens joined, a
/// blank where space was), a stringized operand, a name defined in an earlier header, a name
/// that spells nothing or no header found, `#include_next` of a macro, and `__has_include` of a
/// macro; the tree, every name's meaning and the output are the spike's.
#[test]
fn expanded_include_names_are_the_spikes() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define H \"a.h\"\n#include H\nA\n"), ("a.h", "#define A a\n")],
        &[("main.c", "#define S <b.h>\n#include S\n#define P(x) <x.h>\n#include P(c)\nB C\n"), ("inc/b.h", "#define B b\n"), ("inc/c.h", "#define C c\n")],
        &[("main.c", "#define Q(x) #x\n#include Q(a.h)\n#define J(d, f) <d/f.h>\n#include J(sub, d)\nA D\n"), ("a.h", "#define A a\n"), ("inc/sub/d.h", "#define D d\n")],
        &[("main.c", "#include \"names.h\"\n#include NAME\n#include SYS\nX Y\n"), ("names.h", "#define NAME \"x.h\"\n#define SYS < y.h >\n"), ("x.h", "#define X x\n"), ("inc/y.h", "#define Y y\n")],
        &[("main.c", "#define SP < sub /d.h>\n#include SP\n#define E\n#include E\n#define M \"missing.h\"\n#include M\nZ\n"), ("inc/sub/d.h", "#define Z z\n")],
        &[("main.c", "#define N <n.h>\n#include N\nV\n"), ("inc/n.h", "#define NEXT <n.h>\n#define V 1\n#include_next NEXT\n"), ("sys/n.h", "#undef V\n#define V 2\n")],
        &[("main.c", "#define H \"a.h\"\n#define G <b.h>\n#define NO <none.h>\n#if __has_include(H) && __has_include(G) && !__has_include(NO)\nyes\n#else\nno\n#endif\n"), ("a.h", ""), ("inc/b.h", "")],
    ];
    for (k, files) in shapes.iter().enumerate() {
        for cpp in [false, true] {
            let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
            let mut io = Search::new(vec![("inc".into(), DirKind::Angle), ("sys".into(), DirKind::System)], move |p: &str| m.get(p).cloned());
            lookups_agree(Config::new(crate::pp::options(cpp)), &mut io, "main.c", cpp, &format!("shape {k}")).unwrap();
        }
    }
}

/// R7 of decision 0467: each case §6 lists, refused until step 7, gives the serial pass's
/// tokens on the device, in C and C++, a round of 2 launches. A call across a stretch's end (its
/// arguments over a `#define`, an `#undef`, a conditional, a `#line`, a pragma; nested; an exit
/// whose `(` follows a directive; in a header), `#line` and line
/// markers (a line alone, a file, a macro's line and file, in an inactive group, in a header,
/// read by `#if`, an escaped name, a name ending in `/`, malformed), `_Pragma` (no
/// tokens: a string, spaced, prefixed, stringized in a macro, named by a macro, malformed, across
/// lines, in an `#if`), `__COUNTER__` (in
/// object-like and function-like bodies, arguments expanded once at their first plain use, one
/// unused, stringized, pasted, nested calls, a call whose `(` follows its name's list, a header),
/// the configuration's
/// tests in text (direct, through macros, an operand a macro's name read raw, across lines, a name
/// with no `(` plain, `__has_include` plain), `__INCLUDE_LEVEL__`
/// (in the main file, a header, a header's header, a macro's body expanded in a header, an
/// `#if`), `__FILE_NAME__` and `__BASE_FILE__` (in the main file and in nested headers in
/// directories, through a macro, stringized and pasted).
#[test]
fn the_refused_cases_are_the_serial_passes() {
    use rerail_c_pp::files::DirKind;
    let shapes: &[(&str, &[(&str, &str)])] = &[
        (
            "a call across a stretch's end",
            &[
                (
                    "main.c",
                    "#define F(x) [x]\n#define G(a, b) a b\nF(1\n#define A 2\nA)\nG(A,\n#undef A\n#define A 3\nA)\n\
                     F(\n#if 1\nyes\n#else\nno\n#endif\n)\n#define H F\nH\n#define X x\n(X)\nG(__LINE__,\n#line 40\n__LINE__) __LINE__\n\
                     F(F(\n#pragma p\n1))\n#include \"h.h\"\nF(z\n#define B b\n)B\n",
                ),
                ("h.h", "F(in_header\n#define C c\nC)\n"),
            ],
        ),
        (
            "#line and line markers",
            &[
                (
                    "main.c",
                    "a __LINE__ __FILE__ __FILE_NAME__\n#line 100\nb __LINE__ __FILE__\n#define LN __LINE__\nLN\n#line 200 \"dir/new.c\"\n\
                     c __LINE__ __FILE__ __FILE_NAME__ __BASE_FILE__\n#if 0\n#line 1 \"never.c\"\n#endif\nc2 __LINE__ __FILE__\n\
                     #define L 300\n#define NAME \"other.h\"\n#line L NAME\nd __LINE__ __FILE__\n#include \"h.h\"\ne __LINE__ __FILE__\n\
                     #if __LINE__ > 250\nbig\n#endif\n# 50 \"marker.c\" 1\nf __LINE__ __FILE__\n#line 7 \"a\\\\b\\\"c.h\"\ng __FILE__ __FILE_NAME__\n\
                     #line 9 \"x/y/\"\nh __FILE__ __FILE_NAME__ __LINE__\n#line\n#line junk\ni __LINE__\n",
                ),
                ("h.h", "h1 __LINE__ __FILE__\n#line 7\nh2 __LINE__ __FILE__ __INCLUDE_LEVEL__\n"),
            ],
        ),
        (
            "_Pragma",
            &[(
                "main.c",
                "a _Pragma(\"GCC diagnostic push\") b _Pragma ( \"message(\\\"hi\\\")\" ) c\n#define DO(x) _Pragma(#x) x\nDO(GCC warning) d _Pragma(L\"x\") e\n\
                 #define P _Pragma\nP(\"x\") f _Pragma(1) g _Pragma() h _Pragma\ni _Pragma\n(\"y\") j\n#define C __COUNTER__\n_Pragma(\"z\") C\n\
                 #if _Pragma(\"w\") 1\nk\n#endif\n",
            )],
        ),
        (
            "__COUNTER__ in the serial reading's order",
            &[
                (
                    "main.c",
                    "__COUNTER__ __COUNTER__\n#define C __COUNTER__\n#define TWO C C\n#define F(x) __COUNTER__ x x C\n#define G(x, y) y x #x\n\
                     TWO F(__COUNTER__) G(C, F(C))\n#define H(x) x\n#define E(x)\nH(H(C) C) E(C) H(\nC)\n#define K(a, b) a ## b b\nK(C, C)\n\
                     #define M H\nM(C) M\n(C)\n#include \"c.h\"\nC __COUNTER__\n",
                ),
                ("c.h", "in_header C TWO\n"),
            ],
        ),
        (
            "the configuration's tests in text",
            &[(
                "main.c",
                "__has_feature(cxx_rtti) __has_feature(nope) __has_builtin(__builtin_expect) [__has_cpp_attribute(clang::fallthrough)]\n\
                 __is_identifier(foo) __has_feature __has_feature (cxx_rtti) __has_feature(\ncxx_rtti\n)\n\
                 #define T __has_feature\n#define F(x) __has_feature(x)\n#define R cxx_rtti\nT(cxx_rtti) F(cxx_rtti) F(R) __has_feature(R) T (R)\n\
                 #define S(x) #x\nS(__has_feature(cxx_rtti)) __has_attribute(F)\n__has_include(\"a.h\") __has_embed(\"a.h\")\n",
            )],
        ),
        (
            "the built-ins of the instance",
            &[
                ("main.c", "#define LV __INCLUDE_LEVEL__\n__INCLUDE_LEVEL__ __FILE_NAME__ __BASE_FILE__\n#include \"sub/a.h\"\nLV\n#if __INCLUDE_LEVEL__ == 0\nmain_level\n#endif\n"),
                ("sub/a.h", "a: __INCLUDE_LEVEL__ __FILE_NAME__ __BASE_FILE__ LV\n#include <deep/b.h>\n#define S(x) #x\n#define CAT(a, b) a ## b\nS(__FILE_NAME__) CAT(x, __INCLUDE_LEVEL__)\n"),
                ("inc/deep/b.h", "b: __INCLUDE_LEVEL__ __FILE_NAME__ __BASE_FILE__ __FILE__ LV\n#if __INCLUDE_LEVEL__ == 2\nlevel_two\n#endif\n"),
            ],
        ),
    ];
    for (what, files) in shapes {
        for cpp in [false, true] {
            let mut cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
            for (t, op, v) in [("__has_feature", "cxx_rtti", 1), ("__has_builtin", "__builtin_expect", 1), ("__has_cpp_attribute", "clang::fallthrough", 201603), ("__has_attribute", "F", 1)] {
                cfg.features.insert((t.to_string(), op.to_string()), v);
            }
            let (got, want, rung, launches) = expanded_in(files, cfg, &[("inc", DirKind::Angle)]);
            same(got.map(|g| show(&g)), Ok(show(&want)), format!("{what}, C++ {cpp}"));
            same(launches, LAUNCHES, format!("{what}: launches (rung {rung})"));
        }
    }
}

/// R6 of decision 0467: between rounds the host only answers requests. The rounds' runner reads
/// no spike and no serial pass, decides nothing about the unit (no tree, no slots, no activity,
/// no definition), and its loop's only calls on the files are the search and the read.
#[test]
fn the_host_only_loads_files_between_rounds() {
    let src = include_str!("rounds.rs");
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    for banned in ["p1_of", "stream::parallel::run", "Pp::new", "settle", "lookups(", "expand(", "expand_", "dir_slots", "active", "Lookup"] {
        assert!(!code.contains(banned), "the rounds' runner names `{banned}`");
    }
    let io_calls: Vec<&str> = ["io.find(", "io.read("].into_iter().filter(|c| code.contains(c)).collect();
    same(io_calls.len(), 2, "the runner's file calls: the search and the read");
    assert!(!code.contains("io.") || code.matches("io.").count() == 3, "the runner calls the files only to search and read (the main file's read, a header's search and read)");
}

/// R3 of decision 0467: the device iterates the `#ifdef` family, the chains and the groups to
/// their fixpoint, and every directive's and stretch's activity is the spike's (checked inside
/// `lookups_agree`), with every name's meaning. Nested chains, `#elifdef` and `#elifndef`,
/// `#else`, a test that depends on a definition made under an earlier test (a chain of them,
/// so the iterations are several), guards, a stray `#endif`, built-ins tested.
#[test]
fn the_groups_are_the_spikes() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::Search;
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#ifdef A\na\n#elifndef B\nb\n#define B\n#else\nc\n#endif\n#ifdef B\nB_on\n#endif\n")],
        &[("main.c", "#define A\n#ifdef A\n#define B\n#ifdef B\n#define C\n#ifndef C\nno\n#else\n#define D\n#endif\n#endif\n#endif\n#ifdef D\nyes\n#endif\n")],
        &[("main.c", "#ifndef X\n#define X\n#ifdef Y\ny\n#elifdef X\nx\n#elifndef Z\nz\n#endif\n#endif\n#ifdef __LINE__\nline\n#endif\n#ifndef\nempty\n#endif\n#endif\n")],
        &[("main.c", "#include \"g.h\"\n#include \"g.h\"\n#ifdef G\ng\n#endif\n#ifdef H\nh\n#endif\n"), ("g.h", "#ifndef G\n#define G\n#ifdef G\n#define H\n#endif\nbody\n#endif\n")],
        &[("main.c", "#if 1\n#ifdef Q\nq\n#else\n#define Q\n#endif\n#elif 1\nnot\n#else\nnot\n#endif\n#ifdef Q\nQ\n#endif\n#if 0\n#define R\n#elif 1\n#ifdef R\nr\n#endif\n#endif\n")],
        &[("main.c", "#ifdef P0\n#else\n#define P1\n#endif\n#ifdef P1\n#define P2\n#endif\n#ifdef P2\n#define P3\n#endif\n#ifdef P3\n#define P4\n#endif\n#ifdef P4\nall\n#endif\n")],
    ];
    for (k, files) in shapes.iter().enumerate() {
        for cpp in [false, true] {
            let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
            let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
            lookups_agree(Config::new(crate::pp::options(cpp)), &mut io, "main.c", cpp, &format!("shape {k}")).unwrap();
        }
    }
}

/// R2 of decision 0467: `push_macro` and `pop_macro` are rows of the device's table, a pop's
/// value the one its push saw, matched as a stack per name (the spike's `resolve_rows`): every
/// name then means on the device what it means in the spike. Nested pushes, a push of a name not
/// defined, a pop with no push, a push and pop across an include, other names between.
#[test]
fn push_and_pop_are_the_spikes() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::Search;
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define A 1\n#pragma push_macro(\"A\")\n#undef A\nA\n#define A 2\nA\n#pragma pop_macro(\"A\")\nA\n#pragma pop_macro(\"A\")\nA\n")],
        &[("main.c", "#define A a1\n#pragma push_macro(\"A\")\n#define A a2\n#pragma push_macro(\"A\")\n#define A a3\nA\n#pragma pop_macro(\"A\")\nA\n#pragma pop_macro(\"A\")\nA\n")],
        &[("main.c", "#pragma push_macro(\"B\")\n#define B 1\nB\n#pragma pop_macro(\"B\")\nB\n#define B 2\nB\n")],
        &[("main.c", "#define C 1\n#pragma pop_macro(\"C\")\nC\n#pragma push_macro(\"C\")\n#pragma pop_macro(\"C\")\nC\n#pragma pop_macro(\"C\")\nC\n")],
        &[("main.c", "#define D 1\n#define E 1\n#pragma push_macro(\"D\")\n#include \"d.h\"\nD E\n#pragma pop_macro(\"D\")\nD E\n"), ("d.h", "#undef D\n#define D 2\n#pragma push_macro(\"E\")\n#undef E\n#pragma pop_macro(\"E\")\nD E\n")],
        &[("main.c", "#define F 1\n#pragma push_macro(\"F\")\n#pragma push_macro(\"F\")\n#undef F\n#pragma pop_macro(\"F\")\nF\n#undef F\n#pragma pop_macro(\"F\")\nF\n#pragma once\n#pragma push_macro(F)\n#pragma other\nF\n")],
    ];
    for (k, files) in shapes.iter().enumerate() {
        for cpp in [false, true] {
            let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
            let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
            lookups_agree(Config::new(crate::pp::options(cpp)), &mut io, "main.c", cpp, &format!("shape {k}")).unwrap();
        }
    }
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

/// A unit expanded on the device (`expanded`): its output spelled, or its refusal bits; the serial
/// pass's output; the rung it ended at, and its launches.
type DeviceRun = (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, u32, usize);

/// The unit of `files` (its main file `main.c`) expanded on the device from rung `rung`: its
/// output spelled, or its refusal bits; and the serial pass's output. `None` where P1 refuses it.
fn expanded(files: &[(&str, &str)], cpp: bool) -> DeviceRun {
    expanded_in(files, rerail_c_pp::Config::new(crate::pp::options(cpp)), &[])
}

/// `expanded` under `cfg`, its search path `dirs`.
fn expanded_in(files: &[(&str, &str)], cfg: rerail_c_pp::Config, dirs: &[(&str, rerail_c_pp::files::DirKind)]) -> DeviceRun {
    let (got, want, rung, launches, _) = expanded_run(files, cfg, dirs);
    (got, want, rung, launches)
}

/// A device run's outputs (`DeviceRun`) and the givings back it made.
type GivingRun = (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, u32, usize, u32);

/// `expanded_in`, and the givings back the run made (decision 0486 §10).
fn expanded_run(files: &[(&str, &str)], cfg: rerail_c_pp::Config, dirs: &[(&str, rerail_c_pp::files::DirKind)]) -> GivingRun {
    use rerail_c_pp::files::Search;
    let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    let dirs: Vec<(String, rerail_c_pp::files::DirKind)> = dirs.iter().map(|&(d, k)| (d.to_string(), k)).collect();
    let mut serial = Spelled::default();
    let m2 = m.clone();
    let mut io = Search::new(dirs.clone(), move |p: &str| m2.get(p).cloned());
    let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let mut units: Vec<(Box<dyn rerail_c_pp::files::Files>, String)> = vec![(Box::new(Search::new(dirs, move |p: &str| m.get(p).cloned())), "main.c".into())];
    let (s, tapes) = settled_many(&cfg, &mut units, &mut Ran::default()).pop().unwrap().unwrap();
    let x = &*s.part.x;
    if x.refused != 0 {
        return (Err(x.refused), serial.0, x.rung, x.launches, x.gives);
    }
    (Ok(part_tokens(&s.part, &s.loaded.files, &tapes)), serial.0, x.rung, x.launches, x.gives)
}

/// Decisions 0468 §4 and 0485 §1: an evaluation is a fixed handful of launches, `pp_setup` (the
/// text inside its P1 iterations) and `pp_check`, whatever the unit's size: an empty unit, one of thousands of tokens and
/// definitions, and one that runs again at a larger rung. A dispatch past the groups the device
/// holds at once is refused (decision 0568); at them, it gives the default grid's output.
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
        let (got, want, rung, launches) = expanded(files, false);
        same(got.map(|g| show(&g)), Ok(show(&want)), what);
        same(launches, LAUNCHES, format!("{what}'s launches (rung {rung})"));
    }
    // 4,096 groups of 256 threads: past what any GPU holds at once.
    use rerail_c_pp::files::Search;
    let m: std::collections::HashMap<String, Vec<u8>> = [("main.c".to_string(), big.into_bytes())].into_iter().collect();
    let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
    let d = on_device(rerail_c_pp::Config::new(crate::pp::options(false)), &mut io, "main.c", false).unwrap();
    let (unit, lay) = (&d.unit, &d.lay);
    // Decision 0568: refused on the host, past the grid kernel's resident groups, before it is encoded.
    let resident = rerail_metal::pp_arena::residency();
    assert!(resident.groups < 4096, "{resident:?}");
    assert!(matches!(rerail_metal::pp_expand::expand_at(unit, lay, 0, 4096), Err(rerail_metal::Error::BackendCapacity)), "a grid past residency is refused");
    // At its residency, it runs: the same output as the grid it dispatches by default.
    let full = rerail_metal::pp_expand::expand_at(unit, lay, 0, resident.groups).unwrap();
    let narrow = rerail_metal::pp_expand::expand_from(unit, lay, 0).unwrap();
    same(full.refused, 0, "the resident grid's refusals");
    same(&full.out, &narrow.out, "the resident grid's output");
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

/// R1, R2 and R4 of decision 0485: a text `_Pragma`'s `push_macro`, `pop_macro` and `once` are
/// rows the iterations read, as the serial pass carries them out. Names after a pop in the text
/// and in an `#if`; an include after `once` skipped; a pop through a macro, from a macro's operand,
/// in a header, in an inactive group, nested with `#pragma push_macro`; a diagnostic pragma and a
/// malformed one making nothing. Each in C and C++, 2 launches. Then the refusals, by name.
#[test]
fn pragma_rows_are_the_serial_passes() {
    use rerail_c_pp::files::DirKind;
    let shapes: &[(&str, &[(&str, &str)])] = &[
        ("a pop in the text", &[("main.c", "#define X 1\n_Pragma(\"push_macro(\\\"X\\\")\")\n#undef X\n#define X 2\nX\n_Pragma(\"pop_macro(\\\"X\\\")\")\nX\n#if X == 1\none\n#endif\n")]),
        ("on one line", &[("main.c", "#define X 1\n_Pragma(\"push_macro(\\\"X\\\")\") X\n#undef X\nX _Pragma(\"pop_macro(\\\"X\\\")\") X\n#ifdef X\ndefined\n#endif\n")]),
        ("once", &[("main.c", "#include \"h.h\"\n#include \"h.h\"\n#include \"g.h\"\n#include \"g.h\"\nend\n"), ("h.h", "_Pragma(\"once\") h\n"), ("g.h", "g\n_Pragma( \"once\" )\n")]),
        ("through a macro", &[("main.c", "#define DO(x) _Pragma(#x)\n#define X 1\nDO(push_macro(\"X\"))\n#undef X\nX\nDO(pop_macro(\"X\"))\nX\n")]),
        (
            "a macro's operand",
            &[("main.c", "#define S \"pop_macro(\\\"X\\\")\"\n#define X 1\n#pragma push_macro(\"X\")\n#undef X\nX _Pragma(S) X\n#define D \"GCC diagnostic push\"\n_Pragma(D) ok\n_Pragma(1 2) m\n#define E\n_Pragma(E) m2\n#define W(s) _Pragma(s)\nW(\"once\") w\n")],
        ),
        ("in a header", &[("main.c", "#define X 1\n#pragma push_macro(\"X\")\n#define X 2\n#include \"h.h\"\nX\n"), ("h.h", "X\n_Pragma(\"pop_macro(\\\"X\\\")\")\n")]),
        ("in an inactive group", &[("main.c", "#define X 1\n#pragma push_macro(\"X\")\n#undef X\n#if 0\n_Pragma(\"pop_macro(\\\"X\\\")\")\n#endif\nX\n")]),
        (
            "nested with the directives",
            &[(
                "main.c",
                "#define X 1\n#pragma push_macro(\"X\")\n#define X 2\n_Pragma(\"push_macro(\\\"X\\\")\")\n#define X 3\nX\n#pragma pop_macro(\"X\")\nX\n_Pragma(\"pop_macro(\\\"X\\\")\")\nX\n",
            )],
        ),
        (
            "a pop that decides an include",
            &[
                ("main.c", "#define X 1\n#pragma push_macro(\"X\")\n#undef X\n_Pragma(\"pop_macro(\\\"X\\\")\")\n#if X\n#include \"a.h\"\n#endif\nA\n"),
                ("a.h", "#define A a\n_Pragma(\"push_macro(\\\"A\\\")\")\n#undef A\nA\n_Pragma(\"pop_macro(\\\"A\\\")\")\n"),
            ],
        ),
    ];
    for (what, files) in shapes {
        for cpp in [false, true] {
            let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
            let (got, want, rung, launches) = expanded_in(files, cfg, &[("inc", DirKind::Angle)]);
            same(got.map(|g| show(&g)), Ok(show(&want)), format!("{what}, C++ {cpp}"));
            same(launches, LAUNCHES, format!("{what}: launches (rung {rung})"));
        }
    }
    // Refused, by name: a name read inside the invocation that pops it, and two rows of one name
    // from one invocation.
    for main in [
        "#define X 1\n#define P _Pragma(\"pop_macro(\\\"X\\\")\") X\n#pragma push_macro(\"X\")\n#undef X\nP\n",
        "#define Q _Pragma(\"push_macro(\\\"X\\\")\") _Pragma(\"pop_macro(\\\"X\\\")\")\nQ\n",
    ] {
        let (got, _, _, _) = expanded(&[("main.c", main)], false);
        same(got.err().map(|r| r & rerail_metal::pp_expand::X_PRAGMA), Some(rerail_metal::pp_expand::X_PRAGMA), format!("{main:?}"));
    }
    // A row-making `_Pragma` in a directive's line.
    let m: std::collections::HashMap<String, Vec<u8>> = [("main.c".to_string(), b"#if _Pragma(\"once\") 1\n#endif\n".to_vec())].into_iter().collect();
    let mut io = rerail_c_pp::files::Search::new(Vec::new(), move |p: &str| m.get(p).cloned());
    same(on_device(rerail_c_pp::Config::new(crate::pp::options(false)), &mut io, "main.c", false).err(), Some("a _Pragma that makes a row"), "a _Pragma's row in an #if");
}

/// R3 of decision 0485: `__COUNTER__` in `#if`, `#elif`, an `#include`'s name and `#line`
/// counts with the text's uses in the serial reading's order: text before and after, two lines,
/// an arm not read, the text's uses feeding an `#if`, a call whose arguments run over a
/// directive that counts, a header's uses. Each in C and C++, 2 launches.
/// An `#if` line longer than its SIMD group's window (`p1_if_line`: 64 tokens a lane's, past them
/// the group's, 32 tokens a step): a token the window does not read (`defined`, `__has_include`, a
/// character constant, an alternative operator) at each place about a window's edge, and after
/// it, the window again. Each as the serial pass, true and false.
#[test]
fn a_long_if_line_is_read_by_its_group() {
    use rerail_c_pp::files::DirKind;
    let head = "#define X
#define T8 1+1+1+1+1+1+1+1
#define T32 T8+T8+T8+T8
";
    let mut lines = vec![
        "T32 + T32 + defined X + defined(Y) == 65".to_string(),
        "T32 + T32 + defined X + defined(Y) == 66".to_string(),
        "T32 && __has_include(<h1.h>) && T32 - 31 == 1".to_string(),
        "T32 && __has_include(<none.h>) || T32 + T32 == 63".to_string(),
        "(T32 * 2 > 63) ? 'a' == 97 : 0".to_string(),
        "T32 - T32 or (T32 and 0) or (0x20 == T32)".to_string(),
        "-T32 + T32 == 0 && !(T32 < 0) && (T32 | 64) == 96".to_string(),
    ];
    for pad in [29, 30, 31, 32, 33, 63, 64, 65, 66] {
        let ones = "1+".repeat(pad);
        lines.push(format!("{ones} defined X + T32 + defined Y == {}", pad + 33));
        lines.push(format!("{ones} 'b' - 'a' + T32 == {}", pad + 32));
    }
    for line in &lines {
        let main = format!("{head}#if {line}
yes
#else
no
#endif
end
");
        for cpp in [false, true] {
            let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
            let (got, want, _, _) = expanded_in(&[("main.c", main.as_str()), ("inc/h1.h", "h\n")], cfg, &[("inc", DirKind::Angle)]);
            same(got.map(|g| show(&g)), Ok(show(&want)), format!("{line}, C++ {cpp}"));
        }
    }
}

#[test]
fn counters_in_directives_are_the_serial_passes() {
    use rerail_c_pp::files::DirKind;
    let shapes: &[(&str, &[(&str, &str)])] = &[
        ("text, a line, text", &[("main.c", "a __COUNTER__\n#if __COUNTER__ == 1\nyes\n#endif\nb __COUNTER__\n")]),
        ("two lines", &[("main.c", "#if __COUNTER__ == 5\nno\n#elif __COUNTER__ == 1\nelif\n#endif\n__COUNTER__\n")]),
        ("arms not read", &[("main.c", "#if 0\n#if __COUNTER__\n#endif\n#endif\n#if 1\nt\n#elif __COUNTER__\n#endif\n__COUNTER__\n")]),
        ("the text feeds an #if", &[("main.c", "#define C __COUNTER__\nC C C\n#if C == 3\nthree\n#endif\nC\n")]),
        ("a call over a directive", &[("main.c", "#define F(x) x\nF(__COUNTER__\n#if __COUNTER__ == 0\nz\n#endif\n) __COUNTER__\n")]),
        ("#line", &[("main.c", "#define L __COUNTER__\n__COUNTER__ __COUNTER__\n#line L\n__LINE__ __COUNTER__\n")]),
        (
            "a header's uses",
            &[
                ("main.c", "__COUNTER__\n#include <h1.h>\nafter __COUNTER__\n"),
                ("inc/h1.h", "in __COUNTER__\n#if __COUNTER__ == 2\ntwo\n#endif\n"),
            ],
        ),
    ];
    for (what, files) in shapes {
        for cpp in [false, true] {
            let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
            let (got, want, rung, launches) = expanded_in(files, cfg, &[("inc", DirKind::Angle)]);
            same(got.map(|g| show(&g)), Ok(show(&want)), format!("{what}, C++ {cpp}"));
            same(launches, LAUNCHES, format!("{what}: launches (rung {rung})"));
        }
    }
    // Refused: `#` or `##` of the value (an include's name made of it, the unique-name idiom).
    use rerail_metal::pp_expand::X_COUNTER_SPELLED;
    for main in ["#define C(a, b) C2(a, b)\n#define C2(a, b) a ## b\nC(n, __COUNTER__)\n", "#define S(x) S2(x)\n#define S2(x) #x\nS(__COUNTER__)\n"] {
        let (got, _, _, _) = expanded(&[("main.c", main)], false);
        same(got.err().map(|r| r & X_COUNTER_SPELLED), Some(X_COUNTER_SPELLED), format!("{main:?}"));
    }
}

/// Decision 0468, batching: units expanded in one batch give each its own serial answer. The same
/// names are defined differently, undefined, or left undefined from unit to unit, so a unit that
/// saw an earlier unit's row would differ; `__FILE__` used after another unit redefined it; one
/// unit the device refuses (a call past its file's end), and that unit alone is refused: each
/// unit is a team of its own (decision 0588 §6), and the batch is one run.
#[test]
fn a_batch_is_its_units() {
    use rerail_c_pp::files::Search;
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define A 1\n#define F(x) x A\nA F(2) B\n")],
        &[("main.c", "A F(3) B\n#define B b\nB\n")],
        &[("main.c", "#define A 2\n#undef F\nF(4) A\n")],
        &[("main.c", "#define __FILE__ x\n__FILE__ __LINE__\n")],
        &[("main.c", "__FILE__ __LINE__ A\n")],
        &[("main.c", "#define F(x) x\nF(1\n")],
        &[("main.c", "#include \"a.h\"\nH K\n"), ("a.h", "#define K k\n#define H(x) x\n")],
        &[("main.c", "K H(1)\n")],
    ];
    let cfg = rerail_c_pp::Config::new(crate::pp::options(false));
    let mut serials = Vec::new();
    let mut units: Vec<(Box<dyn rerail_c_pp::files::Files>, String)> = Vec::new();
    for files in shapes {
        let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
        let mut serial = Spelled::default();
        let m2 = m.clone();
        let mut io = Search::new(vec![], move |p: &str| m2.get(p).cloned());
        let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
        pp.run("main.c");
        drop(pp);
        serials.push(serial.0);
        units.push((Box::new(Search::new(vec![], move |p: &str| m.get(p).cloned())), "main.c".into()));
    }
    let settled: Vec<_> = settled_many(&cfg, &mut units, &mut Ran::default()).into_iter().map(Result::unwrap).collect();
    let mut alone = Vec::new();
    for (k, (serial, (s, tapes))) in serials.iter().zip(&settled).enumerate() {
        if s.part.x.refused != 0 {
            alone.push((k, s.part.x.refused));
            continue;
        }
        same(show(&part_tokens(&s.part, &s.loaded.files, tapes)), show(serial), format!("unit {k}"));
    }
    eprintln!("refused alone: {alone:?}");
    same(alone, vec![(5, rerail_metal::pp_expand::X_ACROSS)], "the units refused: a call past its file's end");
    // Every unit's first round, the refused one's too, was the batch's one run; the unit with an
    // include took two rounds, and its second ran alone.
    let runs: Vec<u64> = settled.iter().map(|(s, _)| s.part.x.run).collect();
    assert!(runs.iter().enumerate().all(|(k, &r)| k == 6 || r == runs[0]), "the units of one round in one run: {runs:?}");
    assert_ne!(runs[6], runs[0], "the include's second round its own run");
    same(settled.iter().map(|(s, _)| s.rounds).collect::<Vec<_>>(), vec![1, 1, 1, 1, 1, 1, 2, 1], "each unit's rounds");
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
            let (got, want, _, n) = expanded(files, cpp);
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
            let (got, want, _, n) = expanded(files, cpp);
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

/// Decision 0613 §3: an argument's expansion is a node, and a body passes it on by reference. A chain
/// of calls each handing its parameter to the next (libtiff's `REPEAT8` and `CASE8`, Boost's
/// iterations) reads the argument's tokens once, where it is expanded, and once where the output takes
/// them: not again at each level's collection, expansion and rescan. So the longest piece's steps
/// grow with the output, not with the argument's length times the depth. The outputs are the serial
/// pass's, including where a node's tokens are read one by one: a name in it that may still be called
/// (`f` with no `(` in the argument, called by the body after it), a comma in it that splits the
/// next call's arguments, and `#` or `##` beside its parameter.
#[test]
fn an_argument_passed_on_is_read_once() {
    let chain = |terms: usize| -> String {
        let op: Vec<String> = (0..terms).map(|k| format!("a{k} X")).collect();
        format!("#define X x\n#define R2(op) op; op\n#define R4(op) R2(op); R2(op)\n#define R8(op) R4(op); R4(op)\n#define C8(n, op) switch (n) {{ case 1: R8(op); case 2: R4(op); }}\nC8(w, {})\n", op.join(" "))
    };
    let mut steps = Vec::new();
    for terms in [4usize, 40] {
        let src = chain(terms);
        let (got, want, _, _, _, n) = expanded_steps(&[("main.c", &src)], false);
        same(show(&got.expect("refused")), show(&want), format!("{terms} terms"));
        steps.push(n);
    }
    // The 36 terms more are read once where the argument is expanded (each `X` an invocation: its name,
    // its body, the run before it, at most 4 steps a term), and are 72 entries more of its expansion
    // (`a` from the text, `x` from `X`'s body), each output 12 times: 864 entries the output takes 32 at a
    // time (27 asks). The levels' own steps do not change. Read again at each level, they were 835 steps
    // more (137 and 972).
    eprintln!("steps: {steps:?}");
    assert!(steps[1] <= steps[0] + 4 * 36 + 27, "the steps grow with the argument times the depth: {steps:?}");
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define f(x) [x]\n#define X f\n#define A(op) op(1) op\n#define B(op) A(op)\nB(X y X) B(X)\n")],
        &[("main.c", "#define X a, b\n#define Y (c, d)\n#define P(x, y) <x|y>\n#define A(op) P(op)\n#define B(op) A(op)\nB(X) B(Y) B((X))\n")],
        &[("main.c", "#define X x\n#define S(a) #a\n#define T(a) S(a) a ## 1 a\n#define A(op) T(op)\nA(X y X) A(X)\n")],
        &[("main.c", "#define I(x) x\n#define A(op) I(I(op)) I(op)\nA(I(1) I(2)) A(A(3 I(4)))\n")],
        &[("main.c", "#define g(x) x\n#define F(op) op op\n#define G(op) F(op)\nG(g g) (1) G(g)(2)\n")],
    ];
    let (tokens, _) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens", shapes.len() * 2);
}

/// Decision 0613 §1: a top call's result kept, and every call with its key read from the memo, read
/// where the first wrote it: `F(1)` 100 times is expanded by a few groups (those that run its first
/// calls at once each miss), and each gives the serial pass's tokens. The macros are dear enough to be
/// looked up (`LP_MEMO_COST` steps: `D`'s), as a cheap one's calls are not.
#[test]
fn a_repeated_invocation_is_expanded_once() {
    let d = "#define A(x) x\n#define D A(a) A(a) A(a) A(a) A(a) A(a) A(a) A(a) A(a) A(a) A(a) A(a)\n";
    let calls: Vec<&str> = (0..100).map(|_| "F(1)").collect();
    let src = format!("{d}#define G(x) [x]\n#define F(x) G(x) G(x x) D\n{}\n", calls.join(" "));
    let (got, want, (hits, kept)) = expanded_memo(&[("main.c", &src)], false);
    same(show(&got.expect("refused")), show(&want), "100 calls");
    eprintln!("memo: {hits} hits, {kept} kept");
    assert!(hits >= 50 && kept <= 20, "F(1) is expanded by a few groups: {hits} hits, {kept} kept");
    // Its arguments' tokens are the hit's own, wherever it is and however spaced.
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define F(a, b) a b a\nF(x,y) F( x , y ) F(x,y)\nF(x,y) F(x, y)\n")],
        &[("main.c", "#define I(x) x\n#define F(x) I(x) I((x))\nF(1) F(1) F(2) F(1)\n")],
        &[("main.c", "#define E()\n#define F(x) #x x ## 1\nE() a E() F(b) F(b) F( b ) F(b)\n")],
    ];
    shapes_agree(shapes);
    // A call in a body whose arguments are the body's own (`P(r, g)`) is one result for every call of
    // the body's macro, whatever its arguments; one whose arguments are a parameter's is the call's.
    // (`Y(1)`'s finds `P` dear, `Y(2)`'s keeps its result, the two after read it.)
    let src = format!("{d}#define P(a, b) <a b> D\n#define Y(x) P(r, g) P(x, x) x\nY(1) Y(2) Y(3) Y(4)\n");
    let (got, want, (hits, kept)) = expanded_memo(&[("main.c", &src)], false);
    same(show(&got.expect("refused")), show(&want), "a body's call");
    eprintln!("a body's call: {hits} hits, {kept} kept");
    assert!(hits >= 2, "P(r, g) is one result: {hits} hits, {kept} kept");
}

/// Decision 0613 §1: the macros disabled where a call is read are its key's: `B(1)` entered at the top
/// and inside `A` (where `A` is painted) are two results.
#[test]
fn a_disabled_macro_paints_alike_in_a_memoized_expansion() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define A(x) B(x) B(x)\n#define B(x) A(x) x\nA(1) B(1) A(1) B(1)\n")],
        &[("main.c", "#define A(x) [B(x)] [B(x)]\n#define B(x) <A(x)>\n#define C(x) B(x) A(x)\nC(1) C(1) B(1) A(1)\n")],
        // A call in a body that ends in a function-like name, its `(` after the body's call.
        &[("main.c", "#define g(x) <x>\n#define f(x) x g\n#define h(x) f(x)(2) f(x) x\nh(1) h(1) (3) h(1)\n")],
    ];
    shapes_agree(shapes);
}

/// Decision 0613 §1: a definition changed between two calls (a directive between them): the second
/// is expanded again; two calls with no directive between are one.
#[test]
fn a_definition_changed_between_uses_is_expanded_again() {
    // `F` dear enough to be looked up (`LP_MEMO_COST`), from its first call on.
    let src = "#define G 1\n#define A G\n#define F() A A A A A A A A A A A A A A A A A A A A\nF()\n#undef G\n#define G 2\nF() F()\n";
    let (got, want, (hits, _)) = expanded_memo(&[("main.c", src)], false);
    same(show(&got.expect("refused")), show(&want), "G changed");
    same(hits, 1, "the hits: the last call's");
}

/// Decision 0613 §1: a result that reads a place's built-in (`__LINE__`, `__COUNTER__`) or the text
/// past its `)` (a function-like name at its end) is not kept.
#[test]
fn place_builtins_are_not_memoized() {
    let src = "#define L(x) x __LINE__\nL(a)\nL(a)\nL(a) L(a)\n";
    let (got, want, (hits, kept)) = expanded_memo(&[("main.c", src)], false);
    same(show(&got.expect("refused")), show(&want), "__LINE__");
    same((hits, kept), (0, 0), "the memo's hits and results");
    let src = "#define g(x) <x>\n#define f(x) x g\nf(1)(2) f(1) f(1)(3) f(1) x\n";
    let (got, want, (hits, kept)) = expanded_memo(&[("main.c", src)], false);
    same(show(&got.expect("refused")), show(&want), "an open tail");
    same((hits, kept), (0, 0), "the memo's hits and results");
    let shapes: &[&[(&str, &str)]] = &[&[("main.c", "#define C(x) x __COUNTER__\nC(a) C(a)\nC(a)\n")]];
    shapes_agree(shapes);
}

/// `expanded_run`, and the memo's hits and results.
fn expanded_memo(files: &[(&str, &str)], cpp: bool) -> (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, (u32, u32)) {
    use rerail_c_pp::files::Search;
    let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
    let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    let mut serial = Spelled::default();
    let m2 = m.clone();
    let mut io = Search::new(Vec::new(), move |p: &str| m2.get(p).cloned());
    let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let mut units: Vec<(Box<dyn rerail_c_pp::files::Files>, String)> = vec![(Box::new(Search::new(Vec::new(), move |p: &str| m.get(p).cloned())), "main.c".into())];
    let (s, tapes) = settled_many(&cfg, &mut units, &mut Ran::default()).pop().unwrap().unwrap();
    let x = &*s.part.x;
    if x.refused != 0 {
        return (Err(x.refused), serial.0, x.memo);
    }
    (Ok(part_tokens(&s.part, &s.loaded.files, &tapes)), serial.0, x.memo)
}

/// `expanded_run`, and the longest piece's steps.
fn expanded_steps(files: &[(&str, &str)], cpp: bool) -> (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, u32, usize, u32, u32) {
    use rerail_c_pp::files::Search;
    let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
    let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    let mut serial = Spelled::default();
    let m2 = m.clone();
    let mut io = Search::new(Vec::new(), move |p: &str| m2.get(p).cloned());
    let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let mut units: Vec<(Box<dyn rerail_c_pp::files::Files>, String)> = vec![(Box::new(Search::new(Vec::new(), move |p: &str| m.get(p).cloned())), "main.c".into())];
    let (s, tapes) = settled_many(&cfg, &mut units, &mut Ran::default()).pop().unwrap().unwrap();
    let x = &*s.part.x;
    if x.refused != 0 {
        return (Err(x.refused), serial.0, x.rung, x.launches, x.gives, x.steps);
    }
    (Ok(part_tokens(&s.part, &s.loaded.files, &tapes)), serial.0, x.rung, x.launches, x.gives, x.steps)
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
        // A paste that spells a definition's name takes that name's ref (decision 0486 §10): it
        // expands, `__LINE__` in it is the call's; one no definition has, a chain whose middle no
        // definition has, one defined after it, a left that is no identifier.
        &[("main.c", "#define C(a, b) a ## b\n#define C3(a, b, c) a ## b ## c\n#define W_1 one __LINE__\n#define W_ two\nC(W_, 1) C(W_, 2) C3(W, _, 1)\nC(\nW_,\n1) C(W, _) C(V_, 1) C(1, W_)\n#define V_1 three\nC(V_, 1)\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R6 of decision 0448, its fourth part: `__VA_OPT__` (C++20, C23): its content where the variadic
/// argument, expanded, has tokens (an argument that expands to nothing has none), else a
/// placemarker (so `##` beside it pastes with nothing); its content's parameters and pastes, and
/// its space given to its content's first token; any number in a body.
#[test]
fn va_opt_is_the_serial_passes() {
    let shapes: &[&[(&str, &str)]] = &[
        &[("main.c", "#define F(a, ...) f(a __VA_OPT__(,) __VA_ARGS__)\nF(1) F(1, 2) F(1, 2, 3) F(1,)\n")],
        &[("main.c", "#define E\n#define G(...) [__VA_OPT__(x __VA_ARGS__ y)]\nG() G(E) G(a) G( a , b )\n")],
        &[("main.c", "#define H(x, ...) x ## __VA_OPT__(_v)\n#define K(x, ...) __VA_OPT__(x ## x)\nH(a) H(a, 1) K(b) K(b, 1)\n")],
        &[("main.c", "#define S(...) __VA_OPT__((__VA_ARGS__)) end\nS() S(()) S(a, (b))\n")],
        // Five in a body (each body leaf marked once, decision 0527 §2: no bound on their count).
        &[("main.c", "#define M(...) __VA_OPT__(a) __VA_OPT__(b) x __VA_OPT__(c) __VA_OPT__(d)__VA_OPT__( e )\nM() M(1) M(())\n")],
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
        .map(|&(n, last)| format!("{n} {:.0}", best(&|| stage_seconds(&unit, &lay, last, 0, 32).unwrap_or(f64::NAN))))
        .collect();
    eprintln!("a batch of {} units (us, cumulative): {}", units.len(), t.join(", "));
    let at: Vec<f64> = (1..=60).map(|k| best(&|| stage_seconds(&unit, &lay, TO_ALL + 1, k, 32).unwrap_or(f64::NAN))).collect();
    let steps: Vec<String> = at.windows(2).enumerate().map(|(k, w)| format!("{}:{:.0}", k + 1, w[1] - w[0])).collect();
    eprintln!("  stopped before phase 1: {:.0} us; phase k's cost: {}", at[0], steps.join(" "));
    // `RERAIL_PP_STAGES_ALL`: every phase's cost (best of 3), to the run's end; with `RERAIL_PP_TRACE` and
    // `RERAIL_PP_TRACE_ALL`, a traced run first names each phase's line.
    // `RERAIL_PP_STOPS`: the run stopped at each of the given phases (best of 9).
    if let Ok(stops) = std::env::var("RERAIL_PP_STOPS") {
        let best9 = |stop: u32| (0..9).map(|_| stage_seconds(&unit, &lay, TO_ALL + 1, stop, 32).unwrap_or(f64::NAN)).fold(f64::MAX, f64::min) * 1e6;
        let t: Vec<String> = stops.split(',').map(|k| format!("{k}:{:.0}", best9(k.parse().unwrap()))).collect();
        eprintln!("STOPS {} whole {:.0}", t.join(" "), best9(u32::MAX));
    }
    if std::env::var_os("RERAIL_PP_STAGES_ALL").is_some() {
        let best3 = |stop: u32| (0..3).map(|_| stage_seconds(&unit, &lay, TO_ALL + 1, stop, 32).unwrap_or(f64::NAN)).fold(f64::MAX, f64::min) * 1e6;
        let whole = best3(u32::MAX);
        let (mut prev, mut near) = (best3(1), 0);
        eprintln!("STOPK whole {whole:.1} us");
        for k in 1..4000u32 {
            let t = best3(k + 1);
            eprintln!("STOPK {k} {:.1}", t - prev);
            prev = t;
            near = if t >= whole * 0.995 { near + 1 } else { 0 };
            if near >= 8 {
                break;
            }
        }
    }
}

/// R8 of decision 0467 (and R10 of 0448): the corpus preprocessed on the device, each file its
/// own unit from its main file alone (empty stubs for the headers it does not have), against the
/// serial pass. The spike is not run: the rounds decide every directive and resolve every
/// include. Every unit gives the serial pass's tokens, none is refused (a refusal is counted by
/// reason, then fails the test), and every round is 2 launches. `RERAIL_PP_FIRST` and
/// `RERAIL_PP_COUNT` choose the files of each language; `RERAIL_PP_STAGES` measures a batch.
#[test]
#[ignore = "the corpus (~/orca/rerail-shared/lean-corpus); run in slices"]
fn the_corpus_preprocesses_on_the_device() {
    use rerail_c_pp::files::DirKind;
    let root = corpus_root();
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
    let first = std::env::var("RERAIL_PP_FIRST").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let mut langs = Vec::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<String> = rd.flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
        paths.sort();
        langs.push((paths.into_iter().skip(first).take(count).collect(), cpp));
    }
    // `RERAIL_PP_AGAIN`: the batches once before, the units' second evaluation the one checked (their teams
    // sized by the loop's work the first measured: `Sizes::work`).
    if std::env::var_os("RERAIL_PP_AGAIN").is_some() {
        eprintln!("the first evaluation:");
        against_serial(langs.clone(), vec![("<stub>".into(), DirKind::After)], true);
        eprintln!("the second:");
    }
    let r = against_serial(langs, vec![("<stub>".into(), DirKind::After)], true);
    same(r.refused, std::collections::BTreeMap::new(), "the units refused");
    assert!(r.same_units == 0 || r.launches == LAUNCHES, "{} launches a round, not {LAUNCHES}", r.launches);
}

/// Boost.Preprocessor's tests (`RERAIL_PP_BOOST`, else ~/orca/rerail-shared/boost-preprocessor),
/// each its own unit with the library's headers to include: every unit gives the serial pass's
/// tokens on the device, none refused, 2 launches a round. The C files as C, the others as C++.
/// `RERAIL_PP_ONLY` (a part of the name), `RERAIL_PP_FIRST` and `RERAIL_PP_COUNT` choose files.
#[test]
#[ignore = "Boost.Preprocessor (~/orca/rerail-shared/boost-preprocessor)"]
fn boost_preprocessor_is_the_serial_passes() {
    use rerail_c_pp::files::DirKind;
    let root = std::env::var("RERAIL_PP_BOOST").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/boost-preprocessor"));
    let only = std::env::var("RERAIL_PP_ONLY").ok();
    let mut paths: Vec<String> = std::fs::read_dir(root.join("test")).unwrap().flatten().map(|e| e.path().to_string_lossy().into_owned())
        .filter(|p| [".c", ".cpp", ".cxx"].iter().any(|x| p.ends_with(x)) && only.as_ref().is_none_or(|o| p.contains(o.as_str()))).collect();
    paths.sort();
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
    let first = std::env::var("RERAIL_PP_FIRST").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let (c, cpp): (Vec<String>, Vec<String>) = paths.into_iter().skip(first).take(count).partition(|p| p.ends_with(".c"));
    let inc = root.join("include").to_string_lossy().into_owned();
    let r = against_serial(vec![(c, false), (cpp, true)], vec![(inc, DirKind::Angle)], false);
    same(r.refused, std::collections::BTreeMap::new(), "the units refused");
    assert!(r.same_units > 0 && r.launches == LAUNCHES, "{} units, {} launches a round", r.same_units, r.launches);
}

/// The launches of an evaluation's command buffer: `pp_setup` and `pp_check` (decisions 0468 §4,
/// 0485 §1).
const LAUNCHES: usize = 2;

/// Decision 0588's Tests: the serial pass over the corpus batch (`the_corpus_preprocesses_on_the_device`'s units,
/// `RERAIL_PP_FIRST` and `RERAIL_PP_COUNT` alike), a thread a core, each taking the next unit: its wall time, best
/// and median of 5, the files read into memory first (the device's time is its GPU time, the files placed). Run in
/// the optimized build (`--profile profiling`).
#[test]
#[ignore = "a measurement over the corpus (~/orca/rerail-shared/lean-corpus)"]
fn measure_the_serial_pass_on_all_cores() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Count(usize);
    impl rerail_c_pp::sink::Sink for Count {
        fn token(&mut self, _t: &rerail_c_pp::macros::Tk, _text: &[u8], _at: Option<rerail_c_pp::sink::At>, _off: u32) {
            self.0 += 1;
        }
    }
    let root = corpus_root();
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
    let first = std::env::var("RERAIL_PP_FIRST").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let mut units: Vec<(String, bool)> = Vec::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<String> = rd.flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
        paths.sort();
        units.extend(paths.into_iter().skip(first).take(count).map(|p| (p, cpp)));
    }
    assert!(!units.is_empty(), "no corpus at {}", root.display());
    let files: std::sync::Arc<std::collections::HashMap<String, Vec<u8>>> =
        std::sync::Arc::new(units.iter().filter_map(|(p, _)| std::fs::read(p).ok().map(|b| (p.clone(), b))).collect());
    let cfgs = [Config::new(crate::pp::options(false)), Config::new(crate::pp::options(true))];
    let dirs = vec![("<stub>".to_string(), DirKind::After)];
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let run = |threads: usize| -> (f64, usize) {
        let next = AtomicUsize::new(0);
        let tokens = AtomicUsize::new(0);
        let t0 = std::time::Instant::now();
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((path, cpp)) = units.get(i) else { break };
                        let f = std::sync::Arc::clone(&files);
                        let mut io = Search::new(dirs.clone(), move |q: &str| f.get(q).cloned().or_else(|| q.starts_with("<stub>").then(Vec::new)));
                        let mut sink = Count(0);
                        let mut pp = rerail_c_pp::Pp::new(cfgs[usize::from(*cpp)].clone(), &mut io, &mut sink);
                        pp.run(path);
                        drop(pp);
                        tokens.fetch_add(sink.0, Ordering::Relaxed);
                    }
                });
            }
        });
        (t0.elapsed().as_secs_f64() * 1e3, tokens.load(Ordering::Relaxed))
    };
    for t in [1, threads] {
        let mut ms: Vec<f64> = Vec::new();
        let mut tokens = 0;
        for _ in 0..5 {
            let (m, k) = run(t);
            ms.push(m);
            tokens = k;
        }
        ms.sort_by(f64::total_cmp);
        eprintln!("SERIAL {} units, {tokens} tokens, {t} threads: best {:.2} ms, median {:.2} ms", units.len(), ms[0], ms[2]);
    }
}

/// What a memo and waves of expansions would find in the corpus (decision 0613; `RERAIL_PP_FIRST`,
/// `RERAIL_PP_COUNT` as above): each unit's macro invocations through the serial pass, keyed by what
/// their expansions depend on (`Sink::entered`). The keys bound a memo's hits: `key` (the spellings
/// and the disabled names) from above, `epoch_key` (no definition changed since) from below. The
/// depth a miss is entered at is a wave its expansion waits for.
#[test]
#[ignore = "a measurement over the corpus (~/orca/rerail-shared/lean-corpus)"]
fn measure_what_expansions_repeat() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    use std::collections::{HashMap, HashSet};
    #[derive(Default)]
    struct Rec(Vec<rerail_c_pp::sink::Entered>);
    impl rerail_c_pp::sink::Sink for Rec {
        fn wants_entries(&self) -> bool {
            true
        }
        fn entered(&mut self, e: rerail_c_pp::sink::Entered) {
            self.0.push(e);
        }
    }
    let root = corpus_root();
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
    let first = std::env::var("RERAIL_PP_FIRST").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let mut units: Vec<(String, bool)> = Vec::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<String> = rd.flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
        paths.sort();
        units.extend(paths.into_iter().skip(first).take(count).map(|p| (p, cpp)));
    }
    assert!(!units.is_empty(), "no corpus at {}", root.display());
    let cfgs = [Config::new(crate::pp::options(false)), Config::new(crate::pp::options(true))];
    let dirs = vec![("<stub>".to_string(), DirKind::After)];
    // Across the units: the keys seen, and their invocations' sizes.
    let mut all_keys: HashSet<u64> = HashSet::new();
    let (mut n_all, mut n_unit_key, mut n_unit_epoch, mut n_cross) = (0usize, 0usize, 0usize, 0usize);
    let (mut size_all, mut size_unit_epoch_miss) = (0u64, 0u64);
    let mut depth_hist = [0usize; 12];
    let mut rows: Vec<(usize, usize, usize, u32, u32, String)> = Vec::new();
    for (path, cpp) in &units {
        let mut io = Search::new(dirs.clone(), |q: &str| std::fs::read(q).ok().or_else(|| q.starts_with("<stub>").then(Vec::new)));
        let mut rec = Rec::default();
        {
            let mut pp = rerail_c_pp::Pp::new(cfgs[usize::from(*cpp)].clone(), &mut io, &mut rec);
            pp.run(path);
        }
        let e = rec.0;
        let (mut keys, mut epochs): (HashSet<u64>, HashMap<u64, u32>) = (HashSet::new(), HashMap::new());
        let (mut miss_key, mut miss_epoch, mut waves, mut deepest) = (0usize, 0usize, 0u32, 0u32);
        for x in &e {
            n_all += 1;
            size_all += u64::from(x.size);
            depth_hist[(x.depth as usize).min(11)] += 1;
            deepest = deepest.max(x.depth);
            if !all_keys.contains(&x.key) && !keys.contains(&x.key) {
                n_cross += 1;
            }
            if keys.insert(x.key) {
                miss_key += 1;
            }
            if epochs.insert(x.epoch_key, x.depth).is_none() {
                miss_epoch += 1;
                size_unit_epoch_miss += u64::from(x.size);
                waves = waves.max(x.depth);
            }
        }
        all_keys.extend(keys.iter().copied());
        n_unit_key += miss_key;
        n_unit_epoch += miss_epoch;
        rows.push((e.len(), miss_key, miss_epoch, waves, deepest, path.rsplit('/').next().unwrap_or(path).to_string()));
    }
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    eprintln!("EXPANSIONS {} units: {n_all} invocations, {size_all} tokens substituted", units.len());
    eprintln!("EXPANSIONS a unit's memo: distinct by spelling {n_unit_key} ({:.1}% of the invocations), by spelling and no definition changed {n_unit_epoch} ({:.1}%; {:.1}% of the tokens substituted)", pct(n_unit_key, n_all), pct(n_unit_epoch, n_all), 100.0 * size_unit_epoch_miss as f64 / size_all.max(1) as f64);
    eprintln!("EXPANSIONS one memo for the batch, by spelling: {} distinct ({:.1}% of the invocations); {n_cross} invocations new to the batch", all_keys.len(), pct(all_keys.len(), n_all));
    eprintln!("EXPANSIONS invocations by depth entered (0, 1, .., 11+): {depth_hist:?}");
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let only = std::env::var("RERAIL_PP_NAME").ok();
    for (n, mk, me, w, d, name) in rows.iter().filter(|r| only.as_deref().is_none_or(|o| r.5.contains(o))).take(12) {
        eprintln!("EXPANSIONS {name}: {n} invocations, {mk} distinct by spelling, {me} with no definition changed; the deepest miss {w}, the deepest {d}");
    }
}

/// Decision 0613's waves, modelled on the serial pass (`Sink::requests`): each invocation and
/// argument expansion a request, every ready request at once, a unit's memo (a hit computes
/// nothing and waits for the first's result). A request's reading starts when it is posted; its
/// children are posted as its reading reaches them; it is done when its reading and its children
/// are. A child with an open tail holds its parent's reading after it until it is done. Three rules
/// for arguments:
/// - `after`: an invocation's rescan starts once its arguments are expanded;
/// - `holes`: the rescan reads past them, but waits for one whose edge it can't decide
///   (`Request::blocks`), and a call made of an argument's tokens waits for the arguments
///   (`needs_args`);
/// - `free`: neither wait (a bound).
///
/// Costs are counted two ways: tokens, a token read a unit; and steps, 0588's loop modelled
/// (`Request::steps`: a plain run a look-ahead each 32 tokens, a macro's name a step, its arguments a
/// list ask each 32 tokens). The serial pass's cost of a top-level invocation is everything read
/// under it.
///
/// Inputs:
/// - the corpus (`RERAIL_PP_FIRST`, `RERAIL_PP_COUNT`), its headers stubs, or found in
///   `RERAIL_PP_INCLUDE` and `RERAIL_PP_INCLUDE_CPP` (C's and C++'s directories, `:` between) with
///   `RERAIL_PP_PREDEF_C` and `_CPP` (files of `#define` lines, a compiler's predefines); a header
///   found in none is a stub;
/// - Boost.Preprocessor's tests with `RERAIL_PP_SET=boost` (each test once: its `.c`, else its
///   `.cpp`; `RERAIL_PP_ONLY` a part of the name).
///
/// `RERAIL_PP_NAME` filters the rows. `RERAIL_PP_PIECE=A..B` also models the top-level tokens A..B
/// (0588's piece, as `RERAIL_PP_LOOP_LOG` names the slowest) of the units shown.
#[test]
#[ignore = "a measurement over the corpus (~/orca/rerail-shared/lean-corpus)"]
fn measure_the_waves_of_expansions() {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::{DirKind, Search};
    use rerail_c_pp::sink::Request;
    use std::collections::{HashMap, HashSet};
    #[derive(Default)]
    struct Rec(Vec<Request>, Vec<u32>);
    impl rerail_c_pp::sink::Sink for Rec {
        fn wants_entries(&self) -> bool {
            true
        }
        fn requests(&mut self, r: &[Request], deps: &[u32]) {
            self.0 = r.to_vec();
            self.1 = deps.to_vec();
        }
    }
    let env = |k: &str| std::env::var(k).ok();
    let count = env("RERAIL_PP_COUNT").and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
    let first = env("RERAIL_PP_FIRST").and_then(|n| n.parse().ok()).unwrap_or(0);
    let boost = env("RERAIL_PP_SET").is_some_and(|v| v == "boost");
    let mut units: Vec<(String, bool)> = Vec::new();
    let mut dirs: Vec<(String, DirKind)> = Vec::new();
    if boost {
        let root = env("RERAIL_PP_BOOST").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/boost-preprocessor"));
        let only = env("RERAIL_PP_ONLY");
        let mut paths: Vec<String> = std::fs::read_dir(root.join("test")).unwrap().flatten().map(|e| e.path().to_string_lossy().into_owned())
            .filter(|p| p.ends_with(".c") || p.ends_with(".cpp") && !std::path::Path::new(&p.replace(".cpp", ".c")).exists())
            .filter(|p| only.as_ref().is_none_or(|o| p.contains(o.as_str()))).collect();
        paths.sort();
        units.extend(paths.into_iter().skip(first).take(count).map(|p| { let c = !p.ends_with(".c"); (p, c) }));
        dirs.push((root.join("include").to_string_lossy().into_owned(), DirKind::Angle));
    } else {
        let root = corpus_root();
        for (dir, cpp) in [("c", false), ("cpp", true)] {
            let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
            let mut paths: Vec<String> = rd.flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
            paths.sort();
            units.extend(paths.into_iter().skip(first).take(count).map(|p| (p, cpp)));
        }
    }
    let mut dirs_cpp = dirs.clone();
    for (var, to) in [("RERAIL_PP_INCLUDE", &mut dirs), ("RERAIL_PP_INCLUDE_CPP", &mut dirs_cpp)] {
        for d in env(var).iter().flat_map(|v| v.split(':').filter(|d| !d.is_empty()).map(str::to_string).collect::<Vec<_>>()) {
            to.push((d, DirKind::System));
        }
        to.push(("<stub>".to_string(), DirKind::After));
    }
    assert!(!units.is_empty(), "no units");
    let mut cfgs = [Config::new(crate::pp::options(false)), Config::new(crate::pp::options(true))];
    for (k, var) in [(0, "RERAIL_PP_PREDEF_C"), (1, "RERAIL_PP_PREDEF_CPP")] {
        let Some(f) = env(var) else { continue };
        for line in std::fs::read_to_string(&f).unwrap().lines() {
            let Some(rest) = line.strip_prefix("#define ") else { continue };
            let (n, b) = rest.split_once(' ').unwrap_or((rest, ""));
            cfgs[k].predefines.push((n.to_string(), Some(b.to_string())));
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Args {
        After,
        Holes,
        Free,
    }
    struct Model<'a> {
        r: &'a [Request],
        kids: &'a [Vec<u32>],
        place: &'a [bool],
        steps: bool,
        args: Args,
        /// By key: when the first was done, and how long it took.
        memo: HashMap<u64, (u64, u64)>,
        work: u64,
        computed: u64,
        deepest: u32,
    }
    impl Model<'_> {
        fn cost(&self, q: &Request) -> u64 {
            if self.steps { u64::from(q.steps + q.run.div_ceil(32)) } else { u64::from(q.own) }
        }
        fn pos(&self, q: &Request) -> u64 {
            u64::from(if self.steps { q.pos_steps } else { q.pos })
        }
        /// Request `i` posted at `post`, at nesting `depth`: when it is done.
        fn run(&mut self, i: u32, post: u64, depth: u32) -> u64 {
            let q = self.r[i as usize];
            let memoizable = !self.place[i as usize];
            // A hit waits for the first's result, or takes as long as the first took, whichever is
            // sooner (a result is ready when the first is done).
            if memoizable && let Some(&(f, took)) = self.memo.get(&q.key) {
                return f.min(post + took).max(post);
            }
            self.work += self.cost(&q);
            self.computed += 1;
            self.deepest = self.deepest.max(depth);
            let kids = &self.kids[i as usize];
            let (mut start, mut args_done) = (post, post);
            for &a in kids.iter().filter(|&&k| self.r[k as usize].arg) {
                let f = self.run(a, post, depth + 1);
                args_done = args_done.max(f);
                if self.args == Args::Holes && self.r[a as usize].blocks {
                    start = start.max(f);
                }
            }
            if self.args == Args::After {
                start = args_done;
            }
            let mut late = 0u64;
            for &c in kids.iter().filter(|&&k| !self.r[k as usize].arg) {
                let qc = self.r[c as usize];
                let mut at = start + self.pos(&qc) + late;
                if self.args == Args::Holes && qc.needs_args {
                    at = at.max(args_done);
                }
                let f = self.run(c, at, depth + 1);
                if qc.tails > 0 {
                    late = late.max(f - at);
                }
                args_done = args_done.max(f);
            }
            let done = args_done.max(start + self.cost(&q) + late);
            if memoizable {
                self.memo.insert(q.key, (done, done - post));
            }
            done
        }
    }
    /// Arguments as references (decision 0613), run by continuations: a request reads its list;
    /// at each call it reaches, it waits only for the expansions its call's name and shape came from
    /// (`Request::dep_at`: its name, its `(` and `)`, the commas that split it, a producer's
    /// unpaired parentheses), not for its arguments, which the child takes as references. A wait
    /// is a continuation left on the expansion it waits for, enqueued again when that one is done.
    /// A child's arguments are posted when it is, and read as holes. A memo hit waits for the
    /// first's result (or takes as long as the first took). A child with an open tail holds its
    /// parent's reading after it until it is done.
    struct Lazy<'a> {
        r: &'a [Request],
        deps: &'a [u32],
        kids: &'a [Vec<u32>],
        place: &'a [bool],
        /// Each request's subtree: the requests `i..i + size[i]`.
        size: &'a [u32],
        /// By expansion id: its request (`u32::MAX`: none).
        exp_req: &'a [u32],
        memo: HashMap<u64, u32>,
        /// 0 not posted, 1 posted, 2 done, 3 a memo hit (done at `done`).
        state: Vec<u8>,
        start: Vec<u64>,
        done: Vec<u64>,
        next: Vec<u32>,
        late: Vec<u64>,
        pending: Vec<u32>,
        fin: Vec<u64>,
        up: Vec<u32>,
        waiters: Vec<Vec<u32>>,
        queue: std::collections::VecDeque<u32>,
        suspended: u64,
        work: u64,
        computed: u64,
        odd: u64,
    }
    impl<'a> Lazy<'a> {
        fn new(r: &'a [Request], deps: &'a [u32], kids: &'a [Vec<u32>], place: &'a [bool], size: &'a [u32], exp_req: &'a [u32]) -> Self {
            let n = r.len();
            Lazy { r, deps, kids, place, size, exp_req, memo: HashMap::new(), state: vec![0; n], start: vec![0; n], done: vec![0; n], next: vec![0; n], late: vec![0; n], pending: vec![0; n], fin: vec![0; n], up: vec![u32::MAX; n],
                waiters: vec![Vec::new(); n], queue: std::collections::VecDeque::new(), suspended: 0, work: 0, computed: 0, odd: 0 }
        }
        fn cost(&self, i: u32) -> u64 {
            let q = &self.r[i as usize];
            u64::from(q.steps + q.run.div_ceil(32))
        }
        fn under(&self, p: u32, c: u32) -> bool {
            p <= c && c < p + self.size[p as usize]
        }
        /// When expansion `e`'s tokens are final for request `c`: `Err(w)` to wait on `w`.
        fn ready(&mut self, e: u32, c: u32) -> Result<u64, u32> {
            let p = self.exp_req.get(e as usize).copied().unwrap_or(u32::MAX);
            if p == u32::MAX || self.under(p, c) {
                return Ok(0);
            }
            let mut x = p;
            loop {
                match self.state[x as usize] {
                    2 | 3 => return Ok(self.done[x as usize]),
                    1 => return Err(x),
                    _ => {}
                }
                if x == 0 || self.under(x, c) {
                    self.odd += 1;
                    return Ok(0);
                }
                x = self.r[x as usize].parent;
            }
        }
        /// Posts request `c` at `at`, its parent `up` counting it.
        fn post(&mut self, c: u32, at: u64, up: u32) {
            let ci = c as usize;
            self.state[ci] = 1;
            self.start[ci] = at;
            self.pending[ci] = 1;
            self.up[ci] = up;
            if up != u32::MAX {
                self.pending[up as usize] += 1;
            }
            self.work += self.cost(c);
            self.computed += 1;
            for k in 0..self.kids[ci].len() {
                let a = self.kids[ci][k];
                if !self.r[a as usize].arg {
                    continue;
                }
                let key = self.r[a as usize].key;
                if !self.place[a as usize] && let Some(&f) = self.memo.get(&key) && self.state[f as usize] == 2 {
                    let took = self.done[f as usize] - self.start[f as usize];
                    self.state[a as usize] = 3;
                    self.done[a as usize] = self.done[f as usize].min(at + took).max(at);
                    continue;
                }
                if !self.place[a as usize] {
                    self.memo.entry(key).or_insert(a);
                }
                self.post(a, at, c);
            }
            self.queue.push_back(c);
        }
        fn resume(&mut self, i: u32) {
            let ii = i as usize;
            if self.state[ii] != 1 {
                return;
            }
            while (self.next[ii] as usize) < self.kids[ii].len() {
                let c = self.kids[ii][self.next[ii] as usize];
                let (ci, q) = (c as usize, self.r[c as usize]);
                if q.arg {
                    self.next[ii] += 1;
                    continue;
                }
                let mut at = self.start[ii] + u64::from(q.pos_steps) + self.late[ii];
                for k in q.dep_at..q.dep_at + q.dep_n {
                    match self.ready(self.deps[k as usize], c) {
                        Ok(t) => at = at.max(t),
                        Err(w) => {
                            self.waiters[w as usize].push(i);
                            self.suspended += 1;
                            return;
                        }
                    }
                }
                if self.state[ci] == 0 {
                    // A first that is this one's ancestor (an open tail re-entering the expansion
                    // whose list it ended) can't be waited for: computed again.
                    let hit = if self.place[ci] { None } else { self.memo.get(&q.key).copied().filter(|&f| !self.under(f, c)) };
                    match hit {
                        Some(f) if f != c => {
                            if self.state[f as usize] != 2 {
                                self.waiters[f as usize].push(i);
                                self.suspended += 1;
                                return;
                            }
                            let took = self.done[f as usize] - self.start[f as usize];
                            self.state[ci] = 3;
                            self.done[ci] = self.done[f as usize].min(at + took).max(at);
                            self.fin[ii] = self.fin[ii].max(self.done[ci]);
                        }
                        _ => {
                            if !self.place[ci] {
                                self.memo.insert(q.key, c);
                            }
                            self.post(c, at, i);
                        }
                    }
                }
                if q.tails > 0 {
                    if self.state[ci] == 1 {
                        self.waiters[ci].push(i);
                        self.suspended += 1;
                        return;
                    }
                    self.late[ii] = self.late[ii].max(self.done[ci].saturating_sub(at));
                }
                self.next[ii] += 1;
            }
            let own = self.start[ii] + self.cost(i) + self.late[ii];
            self.fin[ii] = self.fin[ii].max(own);
            self.pending[ii] -= 1;
            if self.pending[ii] == 0 {
                self.complete(i);
            }
        }
        fn complete(&mut self, i: u32) {
            let mut i = i;
            loop {
                let ii = i as usize;
                self.state[ii] = 2;
                self.done[ii] = self.fin[ii];
                let w = std::mem::take(&mut self.waiters[ii]);
                self.queue.extend(w);
                let up = self.up[ii];
                if up == u32::MAX {
                    return;
                }
                let u = up as usize;
                self.fin[u] = self.fin[u].max(self.done[ii]);
                self.pending[u] -= 1;
                if self.pending[u] != 0 {
                    return;
                }
                i = up;
            }
        }
        /// Runs the calls `tops` of the top, each posted at `at(c)`: the latest done, and the
        /// requests left waiting (none, unless the model is wrong).
        fn run(&mut self, tops: &[u32], at: impl Fn(u32) -> u64) -> (u64, u64) {
            let mut end = 0u64;
            for &c in tops {
                let ci = c as usize;
                let q = self.r[ci];
                if !self.place[ci] && let Some(&f) = self.memo.get(&q.key) {
                    // A repeat at the top: done when the first is (the queue is drained between tops).
                    self.state[ci] = 3;
                    let took = self.done[f as usize] - self.start[f as usize];
                    self.done[ci] = self.done[f as usize].min(at(c) + took).max(at(c));
                    end = end.max(self.done[ci]);
                    continue;
                }
                if !self.place[ci] {
                    self.memo.insert(q.key, c);
                }
                self.post(c, at(c), u32::MAX);
                while let Some(i) = self.queue.pop_front() {
                    self.resume(i);
                }
                end = end.max(self.done[ci]);
            }
            let stuck = self.state.iter().filter(|&&s| s == 1).count() as u64;
            (end, stuck)
        }
    }
    let modes = [(false, Args::After), (false, Args::Holes), (false, Args::Free), (true, Args::After), (true, Args::Holes), (true, Args::Free)];
    let piece: Option<(u32, u32)> = env("RERAIL_PP_PIECE").and_then(|v| v.split_once("..").and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))));
    let only = env("RERAIL_PP_NAME");
    #[derive(Default)]
    struct Sum {
        reqs: u64,
        computed: u64,
        read: u64,
        work: u64,
        read_steps: u64,
        work_steps: u64,
        top: u64,
        tails: u64,
        place: u64,
        blocks: u64,
        edge_first: u64,
        edge_last: u64,
        taken: u64,
        needs: u64,
        args: u64,
        batch_work: u64,
        batch_steps: u64,
        lazy_suspended: u64,
        lazy_stuck: u64,
        lazy_odd: u64,
        lazy_work: u64,
    }
    let mut t = Sum::default();
    let mut batch: HashSet<u64> = HashSet::new();
    // By mode: the longest serial and wave chain of any top-level invocation.
    let (mut worst_serial, mut worst, mut worst_lazy) = ([0u64; 2], [0u64; 6], 0u64);
    let mut rows: Vec<(u64, String)> = Vec::new();
    for (path, cpp) in &units {
        let mut io = Search::new(if *cpp { dirs_cpp.clone() } else { dirs.clone() }, |q: &str| std::fs::read(q).ok().or_else(|| q.starts_with("<stub>").then(Vec::new)));
        let mut rec = Rec::default();
        {
            let mut pp = rerail_c_pp::Pp::new(cfgs[usize::from(*cpp)].clone(), &mut io, &mut rec);
            pp.run(path);
        }
        let (r, deps) = (rec.0, rec.1);
        if r.is_empty() {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let mut kids: Vec<Vec<u32>> = vec![Vec::new(); r.len()];
        for (i, q) in r.iter().enumerate().skip(1) {
            kids[q.parent as usize].push(i as u32);
        }
        let mut place: Vec<bool> = r.iter().map(|q| q.place).collect();
        for i in (1..r.len()).rev() {
            if place[i] {
                place[r[i].parent as usize] = true;
            }
        }
        let steps_of = |q: &Request| u64::from(q.steps + q.run.div_ceil(32));
        let (mut under, mut under_s): (Vec<u64>, Vec<u64>) = (r.iter().map(|q| u64::from(q.own)).collect(), r.iter().map(steps_of).collect());
        for i in (1..r.len()).rev() {
            under[r[i].parent as usize] += under[i];
            under_s[r[i].parent as usize] += under_s[i];
        }
        // A memo across the units, by spelling (a bound: it assumes the lookups alike): what is
        // computed once in the batch, a hit's subtree not.
        {
            let mut stack: Vec<u32> = kids[0].clone();
            t.batch_work += u64::from(r[0].own);
            t.batch_steps += steps_of(&r[0]);
            while let Some(i) = stack.pop() {
                let q = &r[i as usize];
                if !place[i as usize] && !batch.insert(q.skey) {
                    continue;
                }
                t.batch_work += u64::from(q.own);
                t.batch_steps += steps_of(q);
                stack.extend(kids[i as usize].iter().copied());
            }
        }
        let tops = &kids[0];
        let mut chains = [0u64; 6];
        let (mut serial, mut serial_s) = (0u64, 0u64);
        let mut computed = 0u64;
        let (mut work, mut work_s, mut deep) = (0u64, 0u64, 0u32);
        for &c in tops {
            serial = serial.max(under[c as usize]);
            serial_s = serial_s.max(under_s[c as usize]);
        }
        for (k, &(steps, args)) in modes.iter().enumerate() {
            let mut m = Model { r: &r, kids: &kids, place: &place, steps, args, memo: HashMap::new(), work: 0, computed: 0, deepest: 0 };
            for &c in tops {
                chains[k] = chains[k].max(m.run(c, 0, 1));
            }
            if k == 1 {
                (work, computed) = (m.work + u64::from(r[0].own), m.computed);
            }
            if k == 4 {
                (work_s, deep) = (m.work + steps_of(&r[0]), m.deepest);
            }
            worst[k] = worst[k].max(chains[k]);
        }
        let mut size: Vec<u32> = vec![1; r.len()];
        for i in (1..r.len()).rev() {
            size[r[i].parent as usize] += size[i];
        }
        let mut exp_req: Vec<u32> = vec![u32::MAX; r.iter().map(|q| q.exp as usize + 1).max().unwrap_or(1)];
        for (i, q) in r.iter().enumerate() {
            if q.exp != 0 && !q.arg {
                exp_req[q.exp as usize] = i as u32;
            }
        }
        let mut lz = Lazy::new(&r, &deps, &kids, &place, &size, &exp_req);
        lz.state[0] = 1;
        let mut lazy = 0u64;
        for &c in tops {
            let (e, _) = lz.run(&[c], |_| 0);
            lazy = lazy.max(e);
        }
        let stuck = lz.state.iter().filter(|&&s| s == 1).count() as u64 - 1;
        if stuck != 0 && env("RERAIL_PP_WHY").is_some() {
            for (i, st) in lz.state.iter().enumerate().skip(1).filter(|p| *p.1 == 1).take(30) {
                let q = &r[i];
                let waits: Vec<usize> = lz.waiters.iter().enumerate().filter(|w| w.1.contains(&(i as u32))).map(|w| w.0).collect();
                eprintln!("WAVES stuck {name}: request {i} (state {st}, parent {}, arg {}, next {}/{}, pending {}), waiting on {waits:?} (states {:?}; ancestors of it: {:?})", q.parent, q.arg, lz.next[i], kids[i].len(), lz.pending[i], waits.iter().map(|&w| lz.state[w]).collect::<Vec<_>>(), waits.iter().map(|&w| lz.under(w as u32, i as u32)).collect::<Vec<_>>());
                let ds: Vec<(u32, u32, u8)> = (q.dep_at..q.dep_at + q.dep_n).map(|k| { let e = deps[k as usize]; let p = exp_req.get(e as usize).copied().unwrap_or(u32::MAX); (e, p, if p == u32::MAX { 9 } else { lz.state[p as usize] }) }).collect();
                eprintln!("WAVES   its deps (exp, request, state): {ds:?}; its kids {:?} (states {:?}), tails {}", kids[i], kids[i].iter().map(|&k| lz.state[k as usize]).collect::<Vec<_>>(), q.tails);
            }
        }
        t.lazy_suspended += lz.suspended;
        t.lazy_stuck += stuck;
        t.lazy_odd += lz.odd;
        t.lazy_work += lz.work + steps_of(&r[0]);
        worst_lazy = worst_lazy.max(lazy);
        worst_serial[0] = worst_serial[0].max(serial);
        worst_serial[1] = worst_serial[1].max(serial_s);
        let sum = |f: &dyn Fn(&Request) -> u64| -> u64 { r.iter().skip(1).map(f).sum() };
        t.reqs += r.len() as u64 - 1;
        t.computed += computed;
        t.read += under[0];
        t.work += work;
        t.read_steps += under_s[0];
        t.work_steps += work_s;
        t.top += u64::from(r[0].own);
        t.tails += sum(&|q| u64::from(q.tails));
        t.place += place.iter().skip(1).filter(|&&p| p).count() as u64;
        t.blocks += sum(&|q| u64::from(q.blocks));
        t.edge_first += sum(&|q| u64::from(q.edge_first));
        t.edge_last += sum(&|q| u64::from(q.edge_last));
        t.taken += sum(&|q| u64::from(q.taken));
        t.needs += sum(&|q| u64::from(q.needs_args));
        t.args += sum(&|q| u64::from(q.arg));
        if only.as_deref().is_none_or(|o| name.contains(o)) {
            rows.push((serial_s, format!("{name}: the longest invocation in steps {serial_s} serially, waves after/holes/references/free {}/{}/{lazy}/{}; in tokens {serial}, {}/{}/{}; read {} steps, {} with the memo; {} deep; {} continuations left waiting",
                chains[3], chains[4], chains[5], chains[0], chains[1], chains[2], under_s[0], work_s, deep, lz.suspended)));
            if let Some((a, b)) = piece {
                // The piece: the top-level tokens a..b, its calls posted at their steps in it.
                let inside: Vec<u32> = tops.iter().copied().filter(|&c| r[c as usize].pos > a && r[c as usize].pos <= b).collect();
                let base = tops.iter().copied().filter(|&c| r[c as usize].pos <= a).last().map_or(0, |c| r[c as usize].pos_steps);
                let last = inside.last().map_or((a, base), |&c| (r[c as usize].pos, r[c as usize].pos_steps));
                let own_s = u64::from(last.1 - base) + u64::from(b.saturating_sub(last.0)).div_ceil(32);
                let serial_piece: u64 = own_s + inside.iter().map(|&c| under_s[c as usize]).sum::<u64>();
                let mut out = Vec::new();
                for args in [Args::After, Args::Holes, Args::Free] {
                    let mut m = Model { r: &r, kids: &kids, place: &place, steps: true, args, memo: HashMap::new(), work: 0, computed: 0, deepest: 0 };
                    let mut done = own_s;
                    for &c in &inside {
                        done = done.max(m.run(c, u64::from(r[c as usize].pos_steps - base), 1));
                    }
                    out.push(done);
                }
                let mut lz = Lazy::new(&r, &deps, &kids, &place, &size, &exp_req);
                lz.state[0] = 1;
                let (e, _) = lz.run(&inside, |c| u64::from(r[c as usize].pos_steps - base));
                eprintln!("WAVES {name} piece {a}..{b}: {} calls at the top; {serial_piece} steps serially ({own_s} the top's own), waves after/holes/references/free {}/{}/{}/{}", inside.len(), out[0], out[1], e.max(own_s), out[2]);
            }
        }
    }
    let pct = |a: u64, b: u64| 100.0 * a as f64 / b.max(1) as f64;
    eprintln!("WAVES {} units: {} requests ({} arguments), {} computed with a unit's memo ({:.1}%), {} place-dependent", units.len(), t.reqs, t.args, t.computed, pct(t.computed, t.reqs), t.place);
    eprintln!("WAVES read serially: {} tokens ({} the files' own), {} steps; with a unit's memo {:.1}% of the tokens, {:.1}% of the steps; with a batch's memo by spelling {:.1}%, {:.1}%", t.read, t.top, t.read_steps, pct(t.work, t.read), pct(t.work_steps, t.read_steps), pct(t.batch_work, t.read), pct(t.batch_steps, t.read_steps));
    eprintln!("WAVES edges: open tails {}; argument holes whose edge waits {} (before {}, after {}); calls whose name and ( came from different parts {}; calls needing their parent's arguments {}", t.tails, t.blocks, t.edge_first, t.edge_last, t.taken, t.needs);
    eprintln!("WAVES the longest top-level invocation: serially {} steps ({} tokens); waves after/holes/references/free {}/{}/{worst_lazy}/{} steps ({}/{}/-/{} tokens)", worst_serial[1], worst_serial[0], worst[3], worst[4], worst[5], worst[0], worst[1], worst[2]);
    eprintln!("WAVES arguments as references: {} continuations left waiting, {} steps computed ({:.1}% of the serial steps), {} requests stuck, {} odd waits", t.lazy_suspended, t.lazy_work, pct(t.lazy_work, t.read_steps), t.lazy_stuck, t.lazy_odd);
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (_, row) in rows.iter().take(15) {
        eprintln!("WAVES {row}");
    }
}

/// What `against_serial` found.
struct Against {
    same_units: usize,
    launches: usize,
    refused: std::collections::BTreeMap<&'static str, usize>,
}

/// Each language's units (main files) on the device against the serial pass, with the include
/// search `dirs` (`stub`: a header not found is an empty stub). A unit unlike the serial pass's
/// panics, naming its first differing token; the refusals are counted by reason.
fn against_serial(langs: Vec<(Vec<String>, bool)>, dirs: Vec<(String, rerail_c_pp::files::DirKind)>, stub: bool) -> Against {
    use rerail_c_pp::Config;
    use rerail_c_pp::files::Search;
    use rerail_metal::pp_expand::*;
    let mut differ: Vec<String> = Vec::new();
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
        (X_PRAGMA, "a _Pragma that makes a row"),
        (X_COUNTER_SPELLED, "# or ## of __COUNTER__'s value"),
        (X_BARRIER, "a grid barrier"),
        (X_WORK, "the work bound"),
    ];
    let (mut same_units, mut tokens, mut launches, mut rounds) = (0, 0, 0, 0);
    let mut ran = Ran::default();
    let mut gpu: Vec<f64> = Vec::new();
    let mut batches: Vec<usize> = Vec::new();
    let mut serial_s = 0f64;
    let mut refused = std::collections::BTreeMap::<&str, usize>::new();
    let mut named = std::collections::BTreeMap::<String, usize>::new();
    let read = move |q: &str| std::fs::read(q).ok().or_else(|| (stub && q.starts_with("<stub>")).then(Vec::new));
    for (paths, cpp) in langs {
        let builtins = rerail_c_pp::stream::parallel::predefined(&Config::new(crate::pp::options(cpp))).1;
        // Each unit's serial answer, then every unit of the language in batched rounds.
        let cfg = Config::new(crate::pp::options(cpp));
        let mut serials = Vec::new();
        let mut ios: Vec<(Box<dyn rerail_c_pp::files::Files>, String)> = Vec::new();
        for path in paths {
            let mut serial = Spelled::default();
            let mut io = Search::new(dirs.clone(), read);
            let t0 = std::time::Instant::now();
            let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
            pp.run(&path);
            drop(pp);
            serial_s += t0.elapsed().as_secs_f64();
            serials.push(serial.0);
            ios.push((Box::new(Search::new(dirs.clone(), read)), path));
        }
        struct Entry<'a> {
            path: &'a str,
            serial: Vec<Vec<u8>>,
            s: super::rounds::Settled,
            tapes: Vec<Vec<u8>>,
        }
        let mut entries = Vec::new();
        let settled = settled_many(&cfg, &mut ios, &mut ran);
        for ((r, serial), (_, path)) in settled.into_iter().zip(serials).zip(&ios) {
            match r {
                Ok((s, tapes)) => {
                    rounds = rounds.max(s.rounds);
                    entries.push(Entry { path, serial, s, tapes });
                }
                Err(why) => {
                    eprintln!("REFUSED {path}: {why}");
                    *refused.entry(why).or_default() += 1;
                }
            }
        }
        // The measurements' units, placed again (resident still, or placed anew).
        let measured = std::env::var_os("RERAIL_PP_PROFILE").is_some() || std::env::var_os("RERAIL_PP_STAGES").is_some();
        let placed: Vec<Unit> = if measured { entries.iter().map(|e| super::rounds::place(&e.s.loaded.files.iter().map(|f| (f.as_slice(), cpp)).collect::<Vec<_>>()).unwrap()).collect() } else { Vec::new() };
        let units: Vec<_> = placed.iter().zip(&entries).map(|(u, e)| (u, &e.s.lay)).collect();
        // `RERAIL_PP_PROFILE`: each unit alone at the rooms its run settled at: its GPU time to
        // the front's end, the definitions', and the whole setup (best of 3, ms); P1's
        // iterations and text passes; its rooms.
        if std::env::var_os("RERAIL_PP_PROFILE").is_some() {
            use rerail_metal::pp_eval::{TO_ALL, TO_DEFS, TO_FRONT};
            use rerail_metal::pp_expand::{expand_sized, stage_seconds_as};
            for (e, (unit, lay)) in entries.iter().zip(&units) {
                let Ok(x) = expand_sized(unit, lay, &mut Sizes::first(unit, lay), rerail_metal::pp_arena::groups()) else { continue };
                let at = |last: u32, iters: u32| (0..3).map(|_| stage_seconds_as(unit, lay, last, 0, iters, &x).unwrap_or(f64::NAN)).fold(f64::MAX, f64::min) * 1e3;
                let best = |last: u32| at(last, 0);
                let name = e.path.rsplit('/').next().unwrap_or(e.path);
                eprintln!(
                    "PROFILE {name}: front {:.1} defs {:.1} setup {:.1} ms (run {:.1}); rounds {}, iterations {}, text passes {}; files {}, nodes {}, dirs {}, top {}, tokens {}/{}, out {}",
                    best(TO_FRONT), best(TO_DEFS), best(TO_ALL + 1), x.gpu_seconds * 1e3, e.s.rounds, x.passes.0, x.passes.1, unit.len(), x.caps.nodes, x.caps.dirs, x.caps.top, x.room[0].0, x.room[0].1, x.out.len()
                );
                // The setup bounded at k of P1's iterations: each one's cost (the last with the text).
                if std::env::var_os("RERAIL_PP_PROFILE_ITERS").is_some() {
                    let t: Vec<f64> = (1..=x.passes.0).map(|k| at(TO_ALL + 1, k)).collect();
                    let steps: Vec<String> = t.iter().scan(at(TO_DEFS, 0), |prev, &v| { let d = v - *prev; *prev = v; Some(format!("{d:.1}")) }).collect();
                    eprintln!("  iterations' ms: {}", steps.join(" "));
                }
                eprintln!("  rooms (used/cap): {}", rerail_metal::pp_expand::ROOM_STORES.iter().zip(x.room).map(|(n, (u, c))| format!("{n} {u}/{c}")).collect::<Vec<_>>().join(", "));
            }
        }
        // `RERAIL_PP_STAGES`: where a batch's GPU time goes (its first `RERAIL_PP_STAGES` units).
        if let Some(k) = std::env::var("RERAIL_PP_STAGES").ok().and_then(|k| k.parse::<usize>().ok()) {
            batch_stages(&units[..k.min(units.len())]);
        }
        // A batch's units answer with its run (`Expanded::run`): its GPU time once, and its units.
        let mut seen: Vec<u64> = Vec::new();
        for e in &entries {
            let (path, part) = (e.path, &e.s.part);
            let x = &*part.x;
            launches = launches.max(x.launches);
            match seen.iter().position(|&r| r == x.run) {
                Some(b) => {
                    let at = batches.len() - seen.len() + b;
                    batches[at] += 1;
                }
                None => {
                    seen.push(x.run);
                    gpu.push(x.gpu_seconds);
                    batches.push(1);
                }
            }
            // Room counts only alone: with another refusal the unit is not rerun, and the other is why.
            let refused_by = if x.refused & !X_ROOM != 0 { x.refused & !X_ROOM } else { x.refused };
            if refused_by & X_ROOM != 0 {
                let full: Vec<_> = ROOM_STORES.iter().zip(x.room).filter(|(_, (u, c))| u > c).map(|(n, (u, c))| format!("{n} {u}/{c}")).collect();
                eprintln!("ROOM {path} at rung {}: {}", x.rung, full.join(", "));
            }
            // The built-ins a unit refused for them names in its text.
            if refused_by & X_BUILTIN != 0 {
                for name in &builtins {
                    let n = name.as_slice();
                    if e.s.loaded.files.iter().any(|f| f.windows(n.len()).any(|w| w == n)) {
                        *named.entry(String::from_utf8_lossy(n).into_owned()).or_default() += 1;
                    }
                }
            }
            if x.refused != 0 {
                for (bit, why) in reasons {
                    if refused_by & bit != 0 {
                        eprintln!("REFUSED {path}: {why}");
                        *refused.entry(why).or_default() += 1;
                    }
                }
                continue;
            }
            let got = part_tokens(part, &e.s.loaded.files, &e.tapes);
            if got != e.serial {
                let k = got.iter().zip(&e.serial).position(|(a, b)| a != b).unwrap_or(got.len().min(e.serial.len()));
                let near = |v: &[Vec<u8>]| show(&v[k.saturating_sub(8)..(k + 8).min(v.len())]);
                eprintln!("DIFF {path}: token {k} (device {}, serial {})\n  device: {}\n  serial: {}", got.len(), e.serial.len(), near(&got), near(&e.serial));
                differ.push(path.to_string());
                continue;
            }
            same_units += 1;
            tokens += got.len();
        }
    }
    eprintln!("{same_units} units the serial pass's ({tokens} tokens), at most {launches} launches a round, at most {rounds} rounds; refused {refused:?}");
    gpu.sort_by(f64::total_cmp);
    if let Some(&most) = gpu.last() {
        let all: f64 = gpu.iter().sum();
        eprintln!("GPU time: {:.1} ms in all over {} runs (units a run: {batches:?}), median {:.3} ms, most {:.3} ms a run", all * 1e3, gpu.len(), gpu[gpu.len() / 2] * 1e3, most * 1e3);
    }
    eprintln!("every round's runs (the batches' rounds and a split's halves; a run again for room inside one not counted): {} runs, {:.1} ms GPU in all", ran.runs, ran.gpu_seconds * 1e3);
    eprintln!("stores' most use (over the top level's tokens, over the room): {}", rerail_metal::pp_expand::ROOM_STORES.iter().zip(ran.per_top.iter().zip(ran.per_cap)).map(|(n, (t, c))| format!("{n} {t:.3}/{c:.3}")).collect::<Vec<_>>().join(", "));
    eprintln!("the serial pass: {:.1} ms in all", serial_s * 1e3);
    eprintln!("the built-ins in units refused for them: {named:?}");
    assert!(differ.is_empty(), "units unlike the serial pass's: {differ:?}");
    Against { same_units, launches, refused }
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
        // A list of many claims (the walk's doubling over them, decision 0527 §2): covered ones with
        // claims of their own, its own exit's name last; then a claim past the list's end after two
        // live ones; then claims that do not nest (`y`'s, covered, reaches past `g`'s), in a list
        // and through to the top level.
        &[("main.c", "#define f(a) g\n#define g(a) #a\n#define y k((\n#define k(a) {a}\n#define W(z) f(0)( y ) z ) ) f(2)( y ) z ) )\nW(1) end\n")],
        &[("main.c", "#define f(x) g\n#define g(y) [y]\n#define W(z) f(1)(f(2)(3)) f(4)(5) z f(6)(f(7)(8) f(9)) f(1)(2) f(3)(4) f(5)(6) f(7)(8) f\nW(0)(9) end\n")],
        &[("main.c", "#define f(x) g\n#define g(y) [y]\n#define V(z) f(1)(2) z f(3)(4) f(5)\nV(0)(6) end V(f(7))(8)\n")],
        &[("main.c", "#define EMPTY()\n#define DEFER(id) id EMPTY()\n#define OBSTRUCT(...) __VA_ARGS__ DEFER(EMPTY)()\n#define EXPAND(...) __VA_ARGS__\n#define A() 123\nDEFER(A)() EXPAND(DEFER(A)()) OBSTRUCT(A)() EXPAND(OBSTRUCT(A)()) EXPAND(EXPAND(OBSTRUCT(A)()))\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R8 of decision 0448: a unit past its rung's room runs again at the next rung (`X_ROOM`), and gives the serial
/// pass's tokens there. `text`'s expansion is past rung 0's room of `what`.
fn past_its_room_runs_again_larger(what: &str, text: &str) {
    let files = [("main.c", text)];
    for cpp in [false, true] {
        let (got, want, rung, _) = expanded(&files, cpp);
        let got = got.unwrap_or_else(|why| panic!("{what}: refused {why}"));
        same(show(&got), show(&want), what);
        assert!(rung > 0, "{what}: ran at rung 0");
        eprintln!("{what}: {} tokens at rung {rung}", got.len());
    }
}

/// R8: an 8⁴ fan-out of a one-token body used three times: 12,288 expansions, each a piece of the output
/// (decision 0588: the loop's room), and more than rung 0's arena of tokens.
#[test]
fn a_unit_past_its_token_room_runs_again_larger() {
    let fan = "#define A0 x\n#define A1 A0 A0 A0 A0 A0 A0 A0 A0\n#define A2 A1 A1 A1 A1 A1 A1 A1 A1\n#define A3 A2 A2 A2 A2 A2 A2 A2 A2\n#define A4 A3 A3 A3 A3 A3 A3 A3 A3\n";
    past_its_room_runs_again_larger("tokens", &format!("{fan}A4 A4 A4\n"));
}

/// R8: a `#` of 512 long names wants more scratch bytes than rung 0's 64 KB.
#[test]
fn a_unit_past_its_scratch_room_runs_again_larger() {
    // 512 names of 256 bytes: 131 KB stringized.
    let name = "n".repeat(256);
    let bytes = format!("#define B0 {name} {name} {name} {name} {name} {name} {name} {name}\n#define B1 B0 B0 B0 B0 B0 B0 B0 B0\n#define B2 B1 B1 B1 B1 B1 B1 B1 B1\n#define S(a) #a\n#define X(a) S(a)\nX(B2) end\n");
    past_its_room_runs_again_larger("scratch bytes", &bytes);
}

/// R8: an `#if` line past rung 0's room that decides an include. The rounds and the expansion's run both run it
/// again larger before reading what the lines decided (a request made from a line cut short is none). Two 8⁴
/// fan-outs: 16,383 pieces of the line's output (decision 0588: the loop's room).
#[test]
fn a_line_past_its_room_runs_again_larger() {
    let fan = "#define A0 1+1+1+1+1+1+1+1\n#define A1 A0+A0+A0+A0+A0+A0+A0+A0\n#define A2 A1+A1+A1+A1+A1+A1+A1+A1\n\
               #define A3 A2+A2+A2+A2+A2+A2+A2+A2\n#define A4 A3+A3+A3+A3+A3+A3+A3+A3\n";
    let main = format!("{fan}#if A4 + A4 == 65536\n#include \"h.h\"\n#endif\nend\n");
    for cpp in [false, true] {
        let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
        let (got, want, rung, _) = expanded_in(&[("main.c", main.as_str()), ("h.h", "in h\n")], cfg, &[]);
        same(got.map(|g| show(&g)), Ok("in h end".to_string()), "a line past its room");
        same(show(&want), "in h end".to_string(), "the serial pass");
        assert!(rung > 0, "a line past its room: ran at rung 0");
    }
}

/// Decision 0486 §9: an argument with no macro's, built-in's or call's name in it is read in
/// place where the body uses it, with no list made for it. Used expanded, once and twice, with
/// `#` and `##` beside, empty, with a painted name in it, with a function-like macro's name
/// (scanned: not flat), as the last of a variadic call and under `__VA_OPT__`, and through levels
/// of calls each passing it on.
#[test]
fn a_flat_argument_is_read_in_place() {
    let shapes: &[&str] = &[
        "#define T(a) [a]\nT(x y z) T( x ) T()\n",
        "#define D(a) a a\nD(1 + 2) D(  p  q  )\n",
        "#define S(a) #a a\n#define P(a, b) a##b a b\nS(x  y) P(l, r) P(, r) P(l,)\n",
        "#define R R(x)\n#define G(a) a R\nG(R) G(z)\n",
        "#define F(x) x(1)\n#define H(n) <n>\nF(H) F(J)\n",
        "#define V(a, ...) a __VA_ARGS__ __VA_OPT__(<__VA_ARGS__>)\nV(1) V(1, 2, 3) V(1,)\n",
        "#define L0(s) s\n#define L1(s) L0(s) L0(s)\n#define L2(s) L1(s) L1((s))\nL2(a b) L2(())\n",
    ];
    for text in shapes {
        let files = [("main.c", *text)];
        for cpp in [false, true] {
            let (got, want, _, _) = expanded(&files, cpp);
            let got = got.unwrap_or_else(|why| panic!("{text:?}: refused {why}"));
            same(show(&got), show(&want), text);
        }
    }
}

/// Decision 0486 §10: what no reader reads again is given back, and used again: the loop gives
/// back what is dead below a substitution's list (decision 0588). Chains of calls each passing a
/// state of 120 tokens on: each gives back, settles at rung 0 and gives the serial pass's tokens. The state passed through
/// plain calls, through an argument scanned at each level (a dead argument's list), and through
/// an exit at each level (a join); `__LINE__` in it (its origin read from the lists' `lref`, not
/// their tokens), `#` of it at the end, and in C and C++.
#[test]
fn reclaimed_tokens_keep_a_chain_at_rung_0() {
    let state = (0..20).map(|i| format!("a{i} (b, c)")).collect::<Vec<_>>().join(" ");
    let levels = 40;
    let chain = |body: &dyn Fn(usize) -> String, last: &str, tail: &str| {
        let mut t = String::from("#define ID(x) x\n#define CAT(a, b) a ## b\n#define S(x) #x\n#define X(x) S(x)\n#define G(x) -\n");
        t += &format!("#define L0(s) {last}\n");
        for k in 1..=levels {
            t += &format!("#define L{k}(s) {}\n", body(k));
        }
        t += &format!("L{levels}({state}{tail})\nend\n");
        t
    };
    let shapes = [
        ("plain", chain(&|k| format!("L{}(s)", k - 1), "[s]", "")),
        ("an argument scanned", chain(&|k| format!("L{}(ID(s))", k - 1), "[s]", "")),
        ("an exit", chain(&|k| format!("CAT(L, {})(s)", k - 1), "<s>", "")),
        ("__LINE__", chain(&|k| format!("L{}(s)", k - 1), "[s]", " __LINE__")),
        ("stringized", chain(&|k| format!("L{}(ID(s))", k - 1), "X(s)", "")),
        ("calls beside it", chain(&|k| format!("G(s) a L{}(s) b G(s) c", k - 1), "[s]", "")),
    ];
    for (what, text) in &shapes {
        let files = [("main.c", text.as_str())];
        for cpp in [false, true] {
            let (got, want, rung, _, gives) = expanded_run(&files, rerail_c_pp::Config::new(crate::pp::options(cpp)), &[]);
            let got = got.unwrap_or_else(|why| panic!("{what}: refused {why}"));
            same(show(&got), show(&want), what);
            assert_eq!(rung, 0, "{what}: grew to rung {rung}");
            assert!(gives > 0, "{what}: gave nothing back");
            eprintln!("{what}: {gives} givings");
        }
    }
}

/// R1 of decision 0520: what a chain holds at once is flat in its length. The chains of
/// `reclaimed_tokens_keep_a_chain_at_rung_0` at 25, 50 and 100 levels, each store's most use
/// (`Exp::room`, as the serial pass's): a store whose use grows with the chain's length holds what
/// no reader reads again, which is what made Boost's units climb the rungs. A pending level keeps
/// a word for its macro, as the serial pass keeps its frame (its macro is disabled until its
/// result is read).
#[test]
fn a_chains_use_is_flat_in_its_length() {
    use rerail_metal::pp_expand::ROOM_STORES;
    let state = (0..20).map(|i| format!("a{i} (b, c)")).collect::<Vec<_>>().join(" ");
    let chain = |levels: usize, body: &dyn Fn(usize) -> String| {
        let mut t = String::from("#define ID(x) x\n#define CAT(a, b) a ## b\n#define L0(s) [s]\n");
        for k in 1..=levels {
            t += &format!("#define L{k}(s) {}\n", body(k));
        }
        t += &format!("L{levels}({state})\nend\n");
        t
    };
    let shapes: [(&str, &dyn Fn(usize) -> String); 3] = [
        ("plain", &|k| format!("L{}(s)", k - 1)),
        ("an argument scanned", &|k| format!("L{}(ID(s))", k - 1)),
        ("an exit", &|k| format!("CAT(L, {})(s)", k - 1)),
    ];
    let mut flat = true;
    for (what, body) in &shapes {
        let uses: Vec<Vec<u32>> = [25, 50, 100]
            .iter()
            .map(|&n| {
                let text = chain(n, body);
                let (got, want, room, _) = expanded_x(&[("main.c", text.as_str())], false);
                let got = got.unwrap_or_else(|why| panic!("{what}, {n} levels: refused {why}"));
                same(show(&got), show(&want), what);
                room.iter().map(|&(used, _)| used).collect()
            })
            .collect();
        for (s, name) in ROOM_STORES.iter().enumerate() {
            // The output and the scratch grow with the input (the state written out once, the
            // literals and the `#line` rows' room, two tokens a directive); the tokens' use at 100
            // levels is at most its use at 25, a 16th more, and a word a level.
            let (a, b, c) = (uses[0][s], uses[1][s], uses[2][s]);
            eprintln!("{what}: {name}: {a} {b} {c}");
            let grows = ["output", "scratch bytes", "scratch tokens"].contains(name);
            let bound = a + a / 16 + 75 + 64;
            if !grows && c > bound {
                flat = false;
            }
        }
    }
    assert!(flat, "a store's use grows with the chain's length (above)");
}

/// A done call's span is given back through its exits' chain, to the chain's last `)` (decision
/// 0588: the substitution gives back what is dead below its list). Each level of a chain has a
/// done call beside its pending one whose exits chain two deep in the level's list
/// (`F2(s)(s)(...)`: `F2` gives `G2`, whose call gives `H2`), the last's arguments 31 pieces. A
/// level adds 64 tokens (its expansion, which the output holds); stopping at the first exit's `)`
/// would keep the last's arguments too (96 a level), so the bound is 80.
#[test]
fn a_done_calls_exits_are_given_back_to_their_last() {
    use rerail_metal::pp_expand::ROOM_STORES;
    let state = (0..20).map(|i| format!("a{i} (b, c)")).collect::<Vec<_>>().join(" ");
    let last = (0..16).map(|_| "s").collect::<Vec<_>>().join(" a ");
    let chain = |levels: usize| {
        let mut t = String::from("#define L0(s) [s]\n#define F2(x) x G2\n#define G2(x) x H2\n#define H2(x) x\n");
        for k in 1..=levels {
            t += &format!("#define L{k}(s) F2(s)(s)({last}) a L{}(s) b\n", k - 1);
        }
        t += &format!("L{levels}({state})\nend\n");
        t
    };
    let tokens = ROOM_STORES.iter().position(|s| *s == "tokens").unwrap();
    let uses: Vec<u32> = [16usize, 48]
        .iter()
        .map(|&n| {
            let text = chain(n);
            let (got, want, room, _) = expanded_x(&[("main.c", text.as_str())], false);
            let got = got.unwrap_or_else(|why| panic!("{n} levels: refused {why}"));
            same(show(&got), show(&want), "a chain of exits beside it");
            room[tokens].0
        })
        .collect();
    eprintln!("tokens: {} {}", uses[0], uses[1]);
    assert!(uses[1] <= uses[0] + 80 * 32, "the tokens grow {} a level", (uses[1] - uses[0]) / 32);
}

/// A unit expanded with its run's own figures (`expanded_x`): its output spelled, or its refusal bits; the serial
/// pass's output; each room's use; the output's pieces.
type RoomsRun = (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, [(u32, u32); rerail_metal::pp_expand::ROOMS], u32);
/// §3 of decision 0520: the output is pieces. A top level's run of tokens no call covers is one
/// piece, and a call's expansion its pieces: a text of 2,000 plain tokens on 200 lines, with a call
/// after each 500 (`G`: `[`, `g`, `]`, `g`, each its own piece of `F`'s body or `G`'s), is its 5 runs
/// and the calls' 4 pieces each, not a word a token, and equals the serial pass.
#[test]
fn the_output_is_pieces_that_refer_to_the_text() {
    let line = (0..10).map(|i| format!("t{i}")).collect::<Vec<_>>().join(" ");
    let mut text = String::from("#define F(x) [x] x\n#define G F(g)\n");
    for k in 0..200 {
        text += &line;
        text += if k % 50 == 49 { " G\n" } else { "\n" };
    }
    let (got, want, _, pieces) = expanded_x(&[("main.c", text.as_str())], false);
    let got = got.unwrap_or_else(|why| panic!("refused {why}"));
    same(show(&got), show(&want), "the text");
    assert!(want.len() > 2000 && pieces <= 5 + 4 * 4, "{} tokens in {pieces} pieces", want.len());
}

/// Decision 0588 §1: the text's expansion is one phase of the loop, whatever its rescans' depth (the rounds took
/// a few phases a rescan): a run's phases are the same for chains of object-like and function-like macros 1, 8 and
/// 20 deep (within the loop's stacks: deeper is refused for depth, `X_DEPTH`), and each output is the serial pass's.
#[test]
fn the_expansion_is_one_loop_phase() {
    use rerail_c_pp::files::Search;
    use rerail_metal::pp_expand::{expand_from, phase_lines};
    let mut runs: Vec<(usize, Vec<u32>)> = Vec::new();
    for depth in [1usize, 8, 20] {
        let mut text = String::from("#define C0 x\n#define F0(a) [a]\n");
        for k in 1..=depth {
            text += &format!("#define C{k} C{}\n#define F{k}(a) F{}(a C{k})\n", k - 1, k - 1);
        }
        text += &format!("int a = C{depth};\nint b = F{depth}(y) + C{};\n", depth / 2);
        let (got, want, _, _) = expanded(&[("main.c", text.as_str())], false);
        same(got.map(|g| show(&g)), Ok(show(&want)), format!("the output at depth {depth}"));
        let m: std::collections::HashMap<String, Vec<u8>> = [("main.c".to_string(), text.into_bytes())].into_iter().collect();
        let mut io = Search::new(vec![], move |p: &str| m.get(p).cloned());
        let d = on_device(rerail_c_pp::Config::new(crate::pp::options(false)), &mut io, "main.c", false).unwrap();
        let x = expand_from(&d.unit, &d.lay, 0).unwrap();
        same(x.refused, 0, format!("refused at depth {depth}"));
        runs.push((depth, phase_lines(&d.unit, &d.lay, &x).unwrap()));
    }
    assert!(!runs[0].1.is_empty(), "a traced run writes its phases");
    for (depth, lines) in &runs[1..] {
        same(lines.len(), runs[0].1.len(), format!("the phases at depth {depth}, against depth 1"));
        same(lines, &runs[0].1, format!("the phases' lines at depth {depth}"));
    }
}

/// Decision 0588: the loop writes its output once, where each piece makes it. A piece that reads
/// past its end (a body's call left open, `#define C F(`, its arguments in the text after it, past
/// the next pieces' starts) takes the pieces it reads, and the pass runs again without them: the
/// output is the serial pass's, with plain text and calls around each, across several pieces'
/// lengths.
#[test]
fn a_piece_read_past_its_end_takes_the_pieces_it_reads() {
    let args = (0..80).map(|i| format!("a{i}")).collect::<Vec<_>>().join(" ");
    let mut text = String::from("#define F(x, y) <x|y>\n#define C F(\n");
    for k in 0..6 {
        text += &format!("p{k} q{k} r{k}\n");
        text += &format!("C {args} , {args} )\n");
        text += "s t u v w\n";
    }
    let (got, want, _, _) = expanded_x(&[("main.c", text.as_str())], false);
    let got = got.unwrap_or_else(|why| panic!("refused {why}"));
    assert!(want.len() > 900, "{} tokens", want.len());
    same(show(&got), show(&want), "the text");
}

/// `expanded`, with the run's own figures (its rooms' use).
fn expanded_x(files: &[(&str, &str)], cpp: bool) -> RoomsRun {
    use rerail_c_pp::files::Search;
    let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
    let m: std::collections::HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    let mut serial = Spelled::default();
    let m2 = m.clone();
    let mut io = Search::new(Vec::new(), move |p: &str| m2.get(p).cloned());
    let mut pp = rerail_c_pp::Pp::new(cfg.clone(), &mut io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let mut units: Vec<(Box<dyn rerail_c_pp::files::Files>, String)> = vec![(Box::new(Search::new(Vec::new(), move |p: &str| m.get(p).cloned())), "main.c".into())];
    let (s, tapes) = settled_many(&cfg, &mut units, &mut Ran::default()).pop().unwrap().unwrap();
    let got = if s.part.x.refused != 0 { Err(s.part.x.refused) } else { Ok(part_tokens(&s.part, &s.loaded.files, &tapes)) };
    (got, serial.0, s.part.x.room, s.part.x.pieces)
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
        // An empty file included: its root has no children (its text map's owner is itself).
        &[("main.c", "#include \"e.h\"\n__FILE__ __LINE__\n#include \"e.h\"\nx __LINE__\n"), ("e.h", "")],
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
        let d = on_device(rerail_c_pp::Config::new(crate::pp::options(false)), &mut io, "main.c", false).unwrap();
        let (unit, lay) = (&d.unit, &d.lay);
        for g in [1u64, 32] {
            let t: Vec<String> = [("front", TO_FRONT), ("defs", TO_DEFS), ("vis", TO_VIS), ("setup", TO_ALL + 1), ("all", TO_ALL)]
                .iter()
                .map(|&(n, last)| format!("{n} {:.0}", best(&|| stage_seconds(unit, lay, last, 0, g).unwrap_or(f64::NAN))))
                .collect();
            eprintln!("{what}, {g} groups (us, cumulative): {}", t.join(", "));
            // Each phase's cost: the run stopped before phase k+1 less the run stopped before k.
            let at: Vec<f64> = (1..=60).map(|k| best(&|| stage_seconds(unit, lay, TO_ALL + 1, k, g).unwrap_or(f64::NAN))).collect();
            let steps: Vec<String> = at.windows(2).enumerate().map(|(k, w)| format!("{}:{:.0}", k + 1, w[1] - w[0])).collect();
            eprintln!("  stopped before phase 1: {:.0} us; phase k's cost: {}", at[0], steps.join(" "));
        }
    }
}
