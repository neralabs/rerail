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
    assert!(src.contains("DEFINE_KERNEL2(pp, pp_setup") && !src.contains("DEFINE_KERNEL2(pp, px_expand") && src.contains("DEFINE_KERNEL2(pp, pp_check"), "the grid's kernels are UKL's");
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
    let unit = place(&files.iter().map(|(s, _, cpp)| (s.as_slice(), *cpp)).collect::<Vec<_>>()).unwrap();
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
    // The rooms carry from round to round (decision 0478 §6).
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

/// The units whose main files are `units`' (each its files), settled in batched rounds
/// (`rounds::run_many`) under `cfg`: each its expansion and its files' tapes, or why the rounds
/// refused it; the runs counted in `ran`.
fn settled_many(cfg: &rerail_c_pp::Config, units: &mut [(Box<dyn rerail_c_pp::files::Files + '_>, String)], ran: &mut Ran) -> Vec<Result<(super::rounds::Settled, Vec<Vec<u8>>), &'static str>> {
    let cpp = cfg.opts.cpp;
    let r = super::rounds::run_many(cfg, cpp, units, |u, l, s| {
        let x = rerail_metal::pp_expand::expand_sized(u, l, s, rerail_metal::pp_arena::groups());
        if let Ok(x) = &x {
            ran.runs += 1;
            ran.gpu_seconds += x.gpu_seconds;
            for (k, &(u, c)) in x.room.iter().enumerate() {
                ran.per_top[k] = ran.per_top[k].max(f64::from(u) / x.caps.top.max(1) as f64);
                ran.per_cap[k] = ran.per_cap[k].max(f64::from(u) / f64::from(c.max(1)));
            }
        }
        x
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

/// The unit of `files` (its main file `main.c`) expanded on the device from rung `rung`: its
/// output spelled, or its refusal bits; and the serial pass's output. `None` where P1 refuses it.
fn expanded(files: &[(&str, &str)], cpp: bool) -> (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, u32, usize) {
    expanded_in(files, rerail_c_pp::Config::new(crate::pp::options(cpp)), &[])
}

/// `expanded` under `cfg`, its search path `dirs`.
fn expanded_in(files: &[(&str, &str)], cfg: rerail_c_pp::Config, dirs: &[(&str, rerail_c_pp::files::DirKind)]) -> (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, u32, usize) {
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
        return (Err(x.refused), serial.0, x.rung, x.launches);
    }
    (Ok(part_tokens(&s.part, &s.loaded.files, &tapes)), serial.0, x.rung, x.launches)
}

/// Decisions 0460 §4 and 0475 §1: an evaluation is a fixed handful of launches, `pp_setup` (the
/// text inside its P1 iterations) and `pp_check`, whatever the unit's size: an empty unit, one of thousands of tokens and
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

/// R1, R2 and R4 of decision 0475: a text `_Pragma`'s `push_macro`, `pop_macro` and `once` are
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

/// R3 of decision 0475: `__COUNTER__` in `#if`, `#elif`, an `#include`'s name and `#line`
/// counts with the text's uses in the serial reading's order: text before and after, two lines,
/// an arm not read, the text's uses feeding an `#if`, a call whose arguments run over a
/// directive that counts, a header's uses. Each in C and C++, 2 launches.
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

/// Decision 0460, batching: units expanded in one batch give each its own serial answer. The same
/// names are defined differently, undefined, or left undefined from unit to unit, so a unit that
/// saw an earlier unit's row would differ; `__FILE__` used after another unit redefined it; one
/// unit the device refuses (a call past its file's end), so the batch splits and that unit alone
/// is refused.
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
    // The units before a refused one ran in a batch of more than one; the unit with an include
    // took two rounds, the others riding with it.
    assert!(std::rc::Rc::ptr_eq(&settled[0].0.part.x, &settled[1].0.part.x), "units 0 and 1 in one batch");
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
        // A paste that spells a definition's name takes that name's ref (decision 0478 §10): it
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

/// The launches of an evaluation's command buffer: `pp_setup` and `pp_check` (decisions 0460 §4,
/// 0475 §1).
const LAUNCHES: usize = 2;

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
                let at = |last: u32, iters: u32| (0..3).map(|_| stage_seconds_as(unit, lay, last, 0, iters, &x).ok().flatten().unwrap_or(f64::NAN)).fold(f64::MAX, f64::min) * 1e3;
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
        let mut seen: Vec<*const Expanded> = Vec::new();
        for e in &entries {
            let (path, part) = (e.path, &e.s.part);
            let x = &*part.x;
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
        &[("main.c", "#define EMPTY()\n#define DEFER(id) id EMPTY()\n#define OBSTRUCT(...) __VA_ARGS__ DEFER(EMPTY)()\n#define EXPAND(...) __VA_ARGS__\n#define A() 123\nDEFER(A)() EXPAND(DEFER(A)()) OBSTRUCT(A)() EXPAND(OBSTRUCT(A)()) EXPAND(EXPAND(OBSTRUCT(A)()))\n")],
    ];
    let (tokens, launches) = shapes_agree(shapes);
    eprintln!("{} shapes, {tokens} tokens, at most {launches} launches an evaluation", shapes.len() * 2);
}

/// R8 of decision 0448: a unit past its rung's room. An 8⁴ fan-out used three times wants more
/// than rung 0's arena of tokens, and a `#` of 512 long names more scratch bytes than rung 0's
/// 64 KB: the device refuses (`X_ROOM`), the unit runs again at the next rung, and it gives the
/// serial pass's tokens there. So does an `#if` line past its room that decides an include.
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
            let (got, want, rung, _) = expanded(&files, cpp);
            let got = got.unwrap_or_else(|why| panic!("{what}: refused {why}"));
            same(show(&got), show(&want), what);
            assert!(rung > 0, "{what}: ran at rung 0");
            eprintln!("{what}: {} tokens at rung {rung}", got.len());
        }
    }
    // An `#if` line past rung 0's room that decides an include: the rounds and the expansion's
    // run both run it again larger before reading what the lines decided (a request made from
    // a line cut short is none).
    let fan = "#define A0 1+1+1+1+1+1+1+1\n#define A1 A0+A0+A0+A0+A0+A0+A0+A0\n#define A2 A1+A1+A1+A1+A1+A1+A1+A1\n\
               #define A3 A2+A2+A2+A2+A2+A2+A2+A2\n#define A4 A3+A3+A3+A3+A3+A3+A3+A3\n";
    let main = format!("{fan}#if A4 == 32768\n#include \"h.h\"\n#endif\nend\n");
    for cpp in [false, true] {
        let cfg = rerail_c_pp::Config::new(crate::pp::options(cpp));
        let (got, want, rung, _) = expanded_in(&[("main.c", main.as_str()), ("h.h", "in h\n")], cfg, &[]);
        same(got.map(|g| show(&g)), Ok("in h end".to_string()), "a line past its room");
        same(show(&want), "in h end".to_string(), "the serial pass");
        assert!(rung > 0, "a line past its room: ran at rung 0");
    }
}

/// Decision 0478 §9: an argument with no macro's, built-in's or call's name in it is read in
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

/// Decision 0478 §10: the expansion's tokens no item reads again are given back between the
/// queue's runs. Chains of 150 calls each passing a state of 120 tokens on make more tokens than
/// rung 0 holds, while few stay live: each settles at rung 0, the serial pass's. The state passed
/// through plain calls, through an argument scanned at each level (a dead argument's list), and
/// through an exit at each level (a join); `__LINE__` in it (its origin read from the lists'
/// `lref`, not their tokens), `#` of it at the end, and in C and C++.
#[test]
fn reclaimed_tokens_keep_a_chain_at_rung_0() {
    let state = (0..20).map(|i| format!("a{i} (b, c)")).collect::<Vec<_>>().join(" ");
    let levels = 150;
    let chain = |body: &dyn Fn(usize) -> String, last: &str, tail: &str| {
        let mut t = String::from("#define ID(x) x\n#define CAT(a, b) a ## b\n#define S(x) #x\n#define X(x) S(x)\n");
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
    ];
    for (what, text) in &shapes {
        let files = [("main.c", text.as_str())];
        for cpp in [false, true] {
            let (got, want, rung, _) = expanded(&files, cpp);
            let got = got.unwrap_or_else(|why| panic!("{what}: refused {why}"));
            same(show(&got), show(&want), what);
            assert_eq!(rung, 0, "{what}: grew to rung {rung}");
        }
    }
}

/// R1 of decision 0520: what a chain holds at once is flat in its length. The chains of
/// `reclaimed_tokens_keep_a_chain_at_rung_0` at 25, 50 and 100 levels, each store's most use
/// (`Exp::room`, as the serial pass's): a store whose use grows with the chain's length holds what
/// no item reads again, which is what made Boost's units climb the rungs.
#[test]
#[ignore = "decision 0520: passes once a list is its pieces (step 2)"]
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
                let (got, want, room) = expanded_x(&[("main.c", text.as_str())], false);
                let got = got.unwrap_or_else(|why| panic!("{what}, {n} levels: refused {why}"));
                same(show(&got), show(&want), what);
                room.iter().map(|&(used, _)| used).collect()
            })
            .collect();
        for (s, name) in ROOM_STORES.iter().enumerate() {
            // The output and the scratch grow with the input (the state written out once, the
            // literals and the `#line` rows' room, two tokens a directive), and the queue's ring is
            // used to half its room between pauses; every other store's use at 100 levels is at
            // most its use at 25 and a 16th more.
            let (a, b, c) = (uses[0][s], uses[1][s], uses[2][s]);
            eprintln!("{what}: {name}: {a} {b} {c}");
            let grows = ["output", "scratch bytes", "scratch tokens", "queue items"].contains(name);
            if !grows && c > a + a / 16 + 64 {
                flat = false;
            }
        }
    }
    assert!(flat, "a store's use grows with the chain's length (above)");
}

/// `expanded`, with the run's own figures (its rooms' use).
fn expanded_x(files: &[(&str, &str)], cpp: bool) -> (Result<Vec<Vec<u8>>, u32>, Vec<Vec<u8>>, [(u32, u32); rerail_metal::pp_expand::ROOMS]) {
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
    (got, serial.0, s.part.x.room)
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
