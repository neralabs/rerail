//! The programs against the serial pass: the spike's cases (`rerail_c_pp::stream::parallel`'s
//! tests), each unit read by the programs on the interpreter and compared token by token.

use super::*;
use rerail_c_pp::files::{DirKind, Search};
use rerail_c_pp::macros::Tk;
use rerail_c_pp::sink::{At, Sink};
use rerail_c_pp::{Options, Pp};

#[derive(Default)]
struct Spellings(Vec<Vec<u8>>);

impl Sink for Spellings {
    fn token(&mut self, _t: &Tk, text: &[u8], _at: Option<At>, _off: u32) {
        self.0.push(text.to_vec());
    }
}

fn io(files: &[(&str, &str)]) -> Search<impl FnMut(&str) -> Option<Vec<u8>>> {
    let m: HashMap<String, Vec<u8>> = files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect();
    Search::new(vec![("sys".into(), DirKind::System)], move |p: &str| m.get(p).cloned())
}

fn show(v: &[Vec<u8>]) -> String {
    v.iter().map(|t| String::from_utf8_lossy(t).into_owned()).collect::<Vec<_>>().join(" ")
}

/// `main.c` of `files` serially and by the programs: the programs' outcome, checked equal to
/// the serial pass where they read it.
fn both(files: &[(&str, &str)]) -> (Outcome, Stats) {
    let cfg = Config::new(Options::c(17, true));
    let mut serial = Spellings::default();
    let mut s_io = io(files);
    let mut pp = Pp::new(cfg.clone(), &mut s_io, &mut serial);
    assert!(pp.run("main.c"));
    drop(pp);
    let (out, stats) = run(cfg.clone(), &mut io(files), "main.c");
    if let Ok(o) = &out {
        assert_eq!(show(o), show(&serial.0), "the programs against the serial pass");
    }
    // The same calls on the words (the interpreter): the same outcome.
    let (w_out, _, _) = run_words(cfg.clone(), &mut io(files), "main.c", rerail_compiler::pp_words::run);
    match (&out, &w_out) {
        (Ok(a), Ok(b)) => assert_eq!(show(b), show(a), "the words' calls against the lean calls"),
        (Err(_), Err(_)) => {}
        _ => panic!("the lean calls: {:?}\nthe words' calls: {:?}", out.as_ref().map(|o| show(o)), w_out.as_ref().map(|o| show(o))),
    }
    // The programs on Cranelift's code and on the device: the interpreter's outcome.
    let mut execs = vec![Exec::Cranelift];
    #[cfg(target_os = "macos")]
    execs.extend([Exec::Metal, Exec::MetalWords]);
    for exec in execs {
        let (m_out, _, _, device) = run_on(cfg.clone(), &mut io(files), "main.c", rerail_compiler::pp_words::run, exec);
        match (&w_out, &m_out) {
            (Ok(a), Ok(b)) => assert_eq!(show(b), show(a), "{exec:?}'s runs against the interpreter's"),
            (Err(a), Err(b)) => assert_eq!(b, a, "{exec:?}'s refusal against the interpreter's"),
            _ => panic!("the interpreter: {:?}\n{exec:?}: {:?}", w_out.as_ref().map(|o| show(o)), m_out.as_ref().map(|o| show(o))),
        }
        assert!(exec == Exec::Cranelift || device.runs > 0 && device.dispatches > 0, "{device:?}");
    }
    (out, stats)
}

fn same_in(files: &[(&str, &str)]) -> Stats {
    let (out, stats) = both(files);
    if let Err(why) = out {
        panic!("refused: {why}");
    }
    stats
}

fn same(src: &str) -> Stats {
    same_in(&[("main.c", src)])
}

#[test]
fn the_programs_verify_and_read_back_from_their_text() {
    for prog in [&*ROUND, &*TEXT] {
        let mut w = rerail_compiler::program::Words::default();
        w.words = vec![rerail_compiler::program::word(rerail_compiler::program::STOP, 0, 0)];
        w.array_programs.push(prog.p.clone());
        let text = rerail_compiler::rir_asm::disassemble(&w);
        let again = rerail_compiler::rir_asm::assemble(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.array_programs[0], prog.p);
    }
}

#[test]
fn macros_object_and_function_like() {
    same("#define N 3\n#define SQ(x) ((x)*(x))\nint a = N + SQ(N + 1);\nint b = SQ(SQ(2));\n");
    same("#define f(a, ...) g(a __VA_OPT__(,) __VA_ARGS__)\nf(1) f(1, 2, 3) f()\n");
    same("#define str(s) # s\n#define xstr(s) str(s)\n#define INCFILE(n) vers ## n\nxstr(INCFILE(2).h) str(strncmp(\"abc\\0d\", \"abc\", '\\4') == 0)\n");
}

#[test]
fn visibility_follows_position() {
    same("#define A 1\nA\n#undef A\nA\n#define A 2\nA\n#define B A\n#undef A\nB\n#define A 3\nB\n");
}

#[test]
fn conditionals_nested_and_chained() {
    same("#define X 2\n#if X == 1\none\n#elif X == 2\ntwo\n#if defined(Y)\ny\n#else\nnot_y\n#endif\n#else\nother\n#endif\nend\n");
    same("#ifdef A\na\n#elifndef B\nnot_b\n#endif\n#ifndef C\n#define C 1\n#endif\n#if C\nc\n#endif\n");
    let st = same("#ifndef CFG\n#define CFG 1\n#endif\n#if CFG\n#define ON 1\n#endif\n#ifdef ON\non\n#endif\n");
    assert!(st.rounds >= 2, "{st:?}");
}

#[test]
fn includes_guards_and_once() {
    let files = [
        ("main.c", "#include \"a.h\"\n#include \"a.h\"\n#include \"o.h\"\n#include \"o.h\"\n#include <s.h>\nA O S\n"),
        ("a.h", "#ifndef A_H\n#define A_H\n#define A from_a\na_body\n#endif\n"),
        ("o.h", "#pragma once\n#define O from_o\no_body\n"),
        ("sys/s.h", "#define S from_s\n#include \"a.h\"\n"),
    ];
    same_in(&files);
}

#[test]
fn an_include_named_by_a_macro_and_has_include() {
    same_in(&[("main.c", "#define H \"h.h\"\n#include H\n#if __has_include(\"h.h\") && !__has_include(\"no.h\")\nyes\n#endif\nV\n"), ("h.h", "#define V 7\n")]);
}

#[test]
fn an_include_switched_by_an_earlier_one() {
    let files = [
        ("main.c", "#include \"cfg.h\"\n#if USE_B\n#include \"b.h\"\n#else\n#include \"a.h\"\n#endif\nWHICH\n"),
        ("cfg.h", "#define USE_B 1\n"),
        ("a.h", "#define WHICH a\n"),
        ("b.h", "#define WHICH b\n"),
    ];
    same_in(&files);
}

#[test]
fn counter_line_and_file() {
    same("#define C __COUNTER__\nC C\n#define two(a, b) a b\ntwo(__COUNTER__, C) __COUNTER__\n");
    same("#define L __LINE__\nL\n#define call(a, b) a b\ncall(L,\n  __LINE__)\n__FILE__ __INCLUDE_LEVEL__\n");
    same_in(&[("main.c", "#include \"i.h\"\n__INCLUDE_LEVEL__\n"), ("i.h", "__FILE__ __INCLUDE_LEVEL__ __LINE__\n")]);
}

#[test]
fn pushes_and_pops() {
    same("#define A 1\n#pragma push_macro(\"A\")\n#undef A\nA\n#define A 2\nA\n#pragma pop_macro(\"A\")\nA\n");
    same("#define min(a, b) m\n#define PUSH _Pragma(\"push_macro(\\\"min\\\")\") _Pragma(\"push_macro(\\\"B\\\")\")\n#define POP _Pragma(\"pop_macro(\\\"min\\\")\") _Pragma(\"pop_macro(\\\"B\\\")\")\nPUSH\n#undef min\nmin(1, 2)\n#if defined(min)\nbad\n#endif\nPOP\nmin(1, 2)\n#ifdef min\nback\n#endif\n");
    same_in(&[("main.c", "#include \"o.h\"\n#include \"o.h\"\nO\n"), ("o.h", "_Pragma(\"once\")\n#define O o\nbody\n")]);
}

/// What the programs do not carry yet is refused, with its reason: merges (none in either
/// corpus), a pop with no push, and what the spike refuses.
#[test]
fn what_is_refused() {
    let refused = |src: &str| both(&[("main.c", src)]).0.err();
    assert_eq!(refused("#define f(x) g\n#define g(y) y + 1\nf(1)(2)\n"), Some("a merge"));
    assert_eq!(refused("#define h(a) [a]\n#define H h(\nH 5) end\n"), Some("a merge"));
    assert_eq!(refused("#define f(x) g\n#define g(y) y\nf(1) + 2\n"), None);
    assert_eq!(refused("#define A 1\n#pragma pop_macro(\"A\")\nA\n"), Some("a pop_macro with no push"));
    assert_eq!(refused("#line 10\n__LINE__\n"), Some("#line"));
    assert_eq!(refused("#if __has_feature(x\nyes\n#endif\nend\n"), None);
    assert_eq!(refused("#define f(a, b) a b\nf(1,\n#ifdef X\n2\n#else\n3\n#endif\n)\n"), Some("a call past its stretch of text"));
    assert_eq!(refused("#if 0\n#line 10\n#endif\nok\n"), None);
    // No directive at all: nothing, and text alone.
    assert_eq!(refused(""), None);
    assert_eq!(refused("a b\n"), None);
}

/// A batch of units read in the same runs: each one's outcome its own read alone, the units
/// at different depths of rounds, one refused in its rounds and one in its text.
#[test]
fn a_batch_reads_each_unit_as_alone() {
    let units: [&[(&str, &str)]; 6] = [
        &[("main.c", "#define A 1\n#if A\nyes\n#else\nno\n#endif\nA\n")],
        &[("main.c", "#include \"a.h\"\nB\n"), ("a.h", "#include \"b.h\"\n#define B C\n"), ("b.h", "#define C 7\n")],
        &[("main.c", "#line 10\n__LINE__\n")],
        &[("main.c", "")],
        &[("main.c", "#define f(x) g\n#define g(y) y + 1\nf(1)(2)\n")],
        &[("main.c", "#define F(a, b) a ## b\nF(x, y) __COUNTER__ __COUNTER__\n#ifndef Z\n#define Z\n#endif\nZ\n")],
    ];
    let cfg = Config::new(Options::c(17, true));
    let alone: Vec<Outcome> = units.iter().map(|f| run_words(cfg.clone(), &mut io(f), "main.c", rerail_compiler::pp_words::run).0).collect();
    assert!(alone.iter().filter(|o| o.is_err()).count() == 2, "{:?}", alone.iter().map(|o| o.as_ref().err()).collect::<Vec<_>>());
    let mut execs = vec![Exec::Interpreter, Exec::Cranelift];
    #[cfg(target_os = "macos")]
    execs.extend([Exec::Metal, Exec::MetalWords]);
    for exec in execs {
        let mut ios: Vec<_> = units.iter().map(|f| io(f)).collect();
        let batch: Vec<(Config, &mut dyn Files, &str)> = ios.iter_mut().map(|io| (cfg.clone(), io as &mut dyn Files, "main.c")).collect();
        let (got, device) = run_batch(batch, rerail_compiler::pp_words::run, exec);
        for (k, (a, (b, _, _))) in alone.iter().zip(&got).enumerate() {
            match (a, b) {
                (Ok(a), Ok(b)) => assert_eq!(show(b), show(a), "unit {k} on {exec:?}"),
                _ => assert_eq!(b.as_ref().err(), a.as_ref().err(), "unit {k} on {exec:?}"),
            }
        }
        if !matches!(exec, Exec::Interpreter | Exec::Cranelift) {
            // A round's run serves every unit still moving: as many runs as the deepest unit's rounds, and the text's.
            let rounds = got.iter().map(|g| g.1.rounds as usize).max().unwrap();
            assert!(device.runs <= rounds + 1, "{} runs for {rounds} rounds", device.runs);
        }
    }
}

/// Both corpora: each file read by the programs against the serial pass, under clang's profile
/// (`rerail_c_pp::oracle_profile`). `RERAIL_PP_LANGS` (`c,cpp`), `RERAIL_PP_FIRST` and
/// `RERAIL_PP_COUNT` choose the files; `RERAIL_PP_EACH` prints each file's verdict and numbers.
#[test]
#[ignore = "the corpus (~/orca/rerail-shared/lean-corpus, or RERAIL_C_CORPUS) and clang"]
fn the_corpus_by_the_programs() {
    use rerail_c_pp::oracle_profile::{Lang, Profile, read_fs};
    let env = |k: &str| std::env::var(k).ok();
    let root = env("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let first: usize = env("RERAIL_PP_FIRST").and_then(|s| s.parse().ok()).unwrap_or(0);
    let count: usize = env("RERAIL_PP_COUNT").and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let each = env("RERAIL_PP_EACH").is_some();
    // The calls on the words' CPU path (`expand::native`), else the lean calls.
    let words = env("RERAIL_PP_WORDS").is_some();
    // The programs on the device (`RERAIL_PP_EXEC=metal`, the calls on the words), else the interpreter.
    #[cfg(target_os = "macos")]
    let exec = match env("RERAIL_PP_EXEC").as_deref() {
        Some("metal") => Exec::Metal,
        Some("metal-words") => Exec::MetalWords,
        Some("cranelift") => Exec::Cranelift,
        _ => Exec::Interpreter,
    };
    #[cfg(not(target_os = "macos"))]
    let exec = Exec::Interpreter;
    let mut results_words: Vec<words::WordStats> = Vec::new();
        let mut device = DeviceStats::default();
    // Units a batch (`run_batch`): their rounds and their text in the same runs.
    let batch: usize = env("RERAIL_PP_BATCH").and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let mut wrong = 0;
    for lang in env("RERAIL_PP_LANGS").unwrap_or("c,cpp".into()).split(',') {
        let mut prof = Profile::new(if lang == "cpp" { Lang::cpp() } else { Lang::c() });
        let mut names: Vec<_> = std::fs::read_dir(root.join(lang)).unwrap().flatten().map(|e| e.path()).collect();
        names.sort();
        let (mut same, mut differ) = (0, 0);
        let mut refused: HashMap<&str, usize> = HashMap::new();
        let chosen: Vec<_> = names.iter().skip(first).take(count).collect();
        for chunk in chosen.chunks(batch) {
            let settled: Vec<_> = chunk.iter().map(|path| prof.settle(&path.to_string_lossy())).collect();
            let mut ios: Vec<_> = settled.iter().map(|s| Search::new(s.dirs.clone(), read_fs)).collect();
            let paths: Vec<String> = chunk.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            let profiled = env("RERAIL_PP_PROFILE").is_some();
            rerail_compiler::array::profile(profiled);
            let t0 = std::time::Instant::now();
            let results: Vec<(Outcome, Stats, words::WordStats)> = if words || exec != Exec::Interpreter {
                let units: Vec<(Config, &mut dyn Files, &str)> = settled.iter().zip(ios.iter_mut()).zip(&paths).map(|((s, io), p)| (s.cfg.clone(), io as &mut dyn Files, p.as_str())).collect();
                let (r, d) = run_batch(units, crate::expand::native, exec);
                device.add(&d);
                r
            } else {
                settled.iter().zip(ios.iter_mut()).zip(&paths).map(|((s, io), p)| { let (o, st) = run(s.cfg.clone(), io, p); (o, st, Default::default()) }).collect()
            };
            let ms = t0.elapsed().as_millis();
            if profiled {
                // The operations by time: program, operation, steps, ms, elements.
                let mut rows: Vec<_> = rerail_compiler::array::profile(false).into_iter().collect();
                rows.sort_by_key(|r| std::cmp::Reverse(r.1.1));
                let total: u64 = rows.iter().map(|r| r.1.1).sum();
                println!("  the operations: {} ms", total / 1_000_000);
                for ((p, i), (n, ns, len)) in rows.iter().take(30) {
                    let (name, prog) = if *p == &ROUND.p as *const _ as usize { ("ROUND", &ROUND.p) } else { ("TEXT", &TEXT.p) };
                    println!("  {name} %{i:<4} {n:>5} steps {:>7.1} ms {len:>10} elements  {}", *ns as f64 / 1e6, prog.ops[*i].text());
                }
            }
            for (((out, st, ws), settled), p) in results.into_iter().zip(&settled).zip(&paths) {
                results_words.push(ws.clone());
                if each {
                    println!("{p}: {ms} ms (its batch); {}; {st:?} {ws:?}", out.as_ref().map_or_else(|w| w.to_string(), |o| if *o == settled.tokens { "same".into() } else { "DIFFERS".into() }));
                }
                match out {
                    Ok(o) if o == settled.tokens => same += 1,
                    Ok(o) => {
                        differ += 1;
                        let k = o.iter().zip(&settled.tokens).position(|(a, b)| a != b).unwrap_or(o.len().min(settled.tokens.len()));
                        let near = |v: &[Vec<u8>]| show(&v[k.saturating_sub(6)..(k + 6).min(v.len())]);
                        println!("DIFF {p}: token {k} (serial {}, programs {})\n  serial:   {}\n  programs: {}", settled.tokens.len(), o.len(), near(&settled.tokens), near(&o));
                    }
                    Err(why) => *refused.entry(why).or_default() += 1,
                }
            }
        }
        println!("{lang}: {same} the same, {differ} differ, refused {refused:?}");
        if exec != Exec::Interpreter {
            println!("  the device: {device:?}");
            let w = |f: fn(&words::WordStats) -> u64| results_words.iter().map(f).sum::<u64>();
            println!("  the words on the device: {} batches, {} dispatches, {:.3} s GPU; {} tests, {} expansions, {} again; words' wall {:.3} s, files' {:.3} s", w(|s| s.batches), w(|s| s.dispatches), w(|s| s.gpu_us) as f64 / 1e6, w(|s| s.tests), w(|s| s.expansions), w(|s| s.again), w(|s| s.run_us) as f64 / 1e6, w(|s| s.make_us) as f64 / 1e6);
            #[cfg(target_os = "macos")]
            println!("  the pool: {:?} blocks and bytes made", rerail_metal::pool::made());
        }
        wrong += differ;
    }
    assert_eq!(wrong, 0, "units the programs read otherwise than the serial pass");
}
