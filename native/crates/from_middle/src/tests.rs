//! The chunk interpreter against the block program's serial one, on rr's, Kotlin's and JSON's
//! grammars.

use crate::chunk::{File, Prepared, Start};
use rerail_compiler::block::BlockProgram;

/// rr's grammar's tier 1.
fn rr() -> &'static BlockProgram {
    &rerail_rr_parser::compiled().expect("rr compiles").block
}

/// Kotlin's grammar's tier 1, without tier 2's Cranelift (most of a compile).
fn kotlin() -> &'static BlockProgram {
    static P: std::sync::OnceLock<BlockProgram> = std::sync::OnceLock::new();
    P.get_or_init(|| rerail_kotlin_parser::block_program(rerail_kotlin_parser::GRAMMAR).expect("Kotlin compiles"))
}

/// JSON in simdjson's structure (`json_parser/json-simdjson.bitlex`).
fn json() -> &'static BlockProgram {
    static P: std::sync::OnceLock<BlockProgram> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let source = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../json_parser/json-simdjson.bitlex")).expect("reads");
        rerail_cpu_stream::ladder::compile_step("json-simdjson", &source).program.block_programs[0].clone()
    })
}

/// The files under `dir` with extension `ext`, sorted, each at most `max` bytes, the first `count`.
fn files_in(dir: &std::path::Path, ext: &str, max: usize, count: usize) -> Vec<Vec<u8>> {
    let (mut paths, mut work) = (Vec::new(), vec![dir.to_path_buf()]);
    while let Some(d) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules", "tmp"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == ext) {
                paths.push(p);
            }
        }
    }
    paths.sort();
    paths.into_iter().filter_map(|p| std::fs::read(p).ok()).filter(|b| b.len() <= max).take(count).collect()
}

/// The repository's `.rerail` files, as rr's own corpus tests read them.
fn rr_files(max: usize, count: usize) -> Vec<Vec<u8>> {
    files_in(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "rerail", max, count)
}

/// `RERAIL_KOTLIN_CORPUS`'s `.kt` files where it is set, and a few of Kotlin's hard cases.
fn kotlin_files(max: usize, count: usize) -> Vec<Vec<u8>> {
    let mut files: Vec<Vec<u8>> = [
        "fun f() {\n    val s = \"a ${b + \"c${d}\"} e\"\n    /* x /* y */ z */\n    val r = \"\"\"raw ${q} \"\" \"\"\"\n}\n",
        "val c = 'x'\nval t = `a b`\n// line \"not a string\n val n = 0x1F + 1e3 + 2.5f\n",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    if let Some(dir) = std::env::var_os("RERAIL_KOTLIN_CORPUS") {
        files.extend(files_in(std::path::Path::new(&dir), "kt", max, count));
    }
    files
}

/// `RERAIL_JSON_CORPUS`'s `.json` files where it is set, and a few documents.
fn json_files(max: usize, count: usize) -> Vec<Vec<u8>> {
    let mut files: Vec<Vec<u8>> = [
        "{\n  \"a\": [1, 2.5e3, -0, true, null],\n  \"b\": \"x\\\"y\\\\\",\n  \"c\": {\"d\": \"\\u00e9\"}\n}\n",
        "[\"a long string with \\\"quotes\\\" and , commas [ ] { } inside it\", 1, 2, 3]",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    if let Some(dir) = std::env::var_os("RERAIL_JSON_CORPUS") {
        files.extend(files_in(std::path::Path::new(&dir), "json", max, count));
    }
    files
}

/// Each file run in chunks of 1, 3 and 16 blocks: chained (each chunk from the one before's
/// end), and then each chunk alone from the start the chain gave it, in a shuffled order. Both
/// equal the serial interpreter's values (its rounds form), every value at every block. A back
/// fill's carry into a chunk's last block is the serial run's (bit 0 of its word at the next
/// block, zero past the file), and each chunk gives the serial run's carry out of its first.
fn chunks_are_the_serial_run(program: &BlockProgram, files: &[Vec<u8>]) {
    let p = Prepared::new(program).expect("a program the chunks run");
    assert!(!files.is_empty(), "files to run");
    for (k, bytes) in files.iter().enumerate() {
        let file = File::new(program, bytes);
        let (want, _) = program.rounds_values(&[bytes]);
        let n = file.count();
        for size in [1usize, 3, 16] {
            let ranges: Vec<std::ops::Range<usize>> = (0..n).step_by(size).map(|c| c..(c + size).min(n)).collect();
            let mut starts: Vec<Start> = Vec::with_capacity(ranges.len());
            let back_at = |i: usize| -> Vec<u64> { p.backs.iter().map(|&x| want[x].get(i).map_or(0, |w| w & 1)).collect() };
            let mut start = p.zero();
            for r in &ranges {
                start.backs = back_at(r.end);
                starts.push(start.clone());
                let (values, end) = p.run(&file, r.clone(), &start);
                for (x, v) in values.iter().enumerate() {
                    assert_eq!(v[..], want[x][r.clone()], "file {k}, chunks of {size}, value {x}, blocks {r:?}, chained");
                }
                assert_eq!(end.backs, back_at(r.start), "file {k}, chunks of {size}, the back fills' carries out of blocks {r:?}");
                start = end;
            }
            // Alone, in an order no chain follows: every chunk reads only its own start.
            let m = ranges.len();
            let mut shuffled: Vec<usize> = (0..m).collect();
            shuffled.sort_by_key(|&i| (i * 7919) % m.max(1));
            shuffled.reverse();
            for i in shuffled {
                let (values, _) = p.run(&file, ranges[i].clone(), &starts[i]);
                for (x, v) in values.iter().enumerate() {
                    assert_eq!(v[..], want[x][ranges[i].clone()], "file {k}, chunks of {size}, value {x}, chunk {i}, alone");
                }
            }
        }
    }
}

#[test]
fn rr_chunks_are_the_serial_run() {
    chunks_are_the_serial_run(rr(), &rr_files(6_000, 12));
}

#[test]
fn kotlin_chunks_are_the_serial_run() {
    chunks_are_the_serial_run(kotlin(), &kotlin_files(6_000, 6));
}

#[test]
fn json_chunks_are_the_serial_run() {
    chunks_are_the_serial_run(json(), &json_files(6_000, 6));
}

/// The serial run's start at every block edge of `file`: each chunk of one block from the end
/// of the one before (the back fills' carries from the serial values, as `chunks_are_the_serial_run`).
fn serial_starts(p: &Prepared<'_>, file: &File, bytes: &[u8]) -> Vec<Start> {
    let (want, _) = p.program.rounds_values(&[bytes]);
    let n = file.count();
    let mut out = Vec::with_capacity(n + 1);
    let mut start = p.zero();
    for i in 0..n {
        start.backs = p.backs.iter().map(|&x| want[x].get(i + 1).map_or(0, |w| w & 1)).collect();
        out.push(start.clone());
        start = p.run(file, i..i + 1, &start).1;
    }
    out.push(start);
    out
}

/// Every component a chunk reads in place, computed from the block before alone, is the serial
/// run's at every block edge.
fn in_place_is_the_serial_run(program: &BlockProgram, files: &[Vec<u8>]) {
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    for (k, bytes) in files.iter().enumerate() {
        let file = File::new(program, bytes);
        for (at, want) in serial_starts(&p, &file, bytes).iter().enumerate().take(file.count()) {
            let mut got = want.clone();
            crate::starts::in_place(&p, &comps, &file, at, &mut got);
            assert_eq!(&got, want, "file {k}, block {at}");
        }
    }
}

#[test]
fn rr_in_place_is_the_serial_run() {
    in_place_is_the_serial_run(rr(), &rr_files(6_000, 12));
}

#[test]
fn kotlin_in_place_is_the_serial_run() {
    in_place_is_the_serial_run(kotlin(), &kotlin_files(6_000, 6));
}

#[test]
fn json_in_place_is_the_serial_run() {
    in_place_is_the_serial_run(json(), &json_files(6_000, 6));
}

/// Each toggle, fill and escape outside the groups: at every 3-block chunk of each file, its
/// words from the other carry in, derived from the base run's words (`derive`), equal a run
/// from the start with that carry flipped.
fn derived_are_the_runs(program: &BlockProgram, files: &[Vec<u8>]) {
    use rerail_compiler::block::BlockOp;
    let p = Prepared::new(program).expect("a program the chunks run");
    let mut checked = 0usize;
    for (k, bytes) in files.iter().enumerate() {
        let file = File::new(program, bytes);
        let starts = serial_starts(&p, &file, bytes);
        for at in (0..file.count()).step_by(3) {
            let r = at..(at + 3).min(file.count());
            let mut base_start = starts[at].clone();
            base_start.backs = starts[r.end - 1].backs.clone();
            for (s, &x) in p.carried.iter().enumerate() {
                let (zero, one) = match program.ops[x] {
                    BlockOp::Toggle(_) => (0, u64::MAX),
                    BlockOp::Fill(..) | BlockOp::Escaped(_) => (0, 1),
                    _ => continue,
                };
                let mut s0 = base_start.clone();
                s0.ops[s] = zero;
                let mut s1 = base_start.clone();
                s1.ops[s] = one;
                let (v0, _) = p.run(&file, r.clone(), &s0);
                let (v1, _) = p.run(&file, r.clone(), &s1);
                let derived = match program.ops[x] {
                    BlockOp::Toggle(_) => crate::derive::toggle_other(&v0[x]),
                    BlockOp::Fill(_, e) => crate::derive::fill_other(&v0[x], &v0[usize::from(e)]),
                    BlockOp::Escaped(a) => crate::derive::escaped_other(&v0[x], &v0[usize::from(a)]),
                    _ => unreachable!(),
                };
                assert_eq!(derived, v1[x], "file {k}, blocks {r:?}, op {x} ({:?})", program.ops[x]);
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "some carry checked");
}

#[test]
fn rr_derived_are_the_runs() {
    derived_are_the_runs(rr(), &rr_files(6_000, 6));
}

#[test]
fn kotlin_derived_are_the_runs() {
    derived_are_the_runs(kotlin(), &kotlin_files(2_000, 2));
}

#[test]
fn json_derived_are_the_runs() {
    derived_are_the_runs(json(), &json_files(6_000, 6));
}

/// A chunk solved from an unknown start (`solve::solve`: its bytes and the bytes before it,
/// nothing else) at every `chunk`-block chunk of each file the program accepts. The serial run
/// only checks the answer: some leaf holds the truth, its entry, end, back fill carries in and
/// out, and outputs each known only where the serial run agrees.
fn solve_is_sound(program: &BlockProgram, files: &[Vec<u8>], chunk: usize, look: usize, cap: usize) -> usize {
    use crate::starts::Place;
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let outputs: Vec<usize> = program.outputs.iter().map(|&o| usize::from(o)).collect();
    let mut chunks = 0;
    for (k, bytes) in files.iter().enumerate() {
        if program.run(bytes).totals.iter().any(|&t| t != 0) {
            continue;
        }
        let file = File::new(program, bytes);
        let starts = serial_starts(&p, &file, bytes);
        let (want, _) = program.rounds_values(&[bytes]);
        for at in (0..file.count()).step_by(chunk) {
            let range = at..(at + chunk).min(file.count());
            let solved = crate::solve::solve(&p, &comps, &file, range.clone(), look, cap);
            // The truth, to check against only.
            let mut entry = starts[at].clone();
            entry.backs = starts[range.end - 1].backs.clone();
            let (tv, _) = p.run(&file, range.clone(), &entry);
            let back_out: Vec<u64> = p.backs.iter().map(|&x| want[x][at] & 1).collect();
            let fwd = |c: &&crate::starts::Component| !c.in_place && !matches!(c.at, Place::Back(_));
            let holds = |l: &crate::solve::Leaf| -> Result<(), String> {
                for c in comps.iter().filter(fwd) {
                    let (e, n) = (l.entry.get(c.at), l.end.get(c.at));
                    if e.v != c.at.get(&starts[at]) & e.k {
                        return Err(format!("entry of op {}", c.op));
                    }
                    if n.v != c.at.get(&starts[range.end]) & n.k {
                        return Err(format!("end of op {}", c.op));
                    }
                }
                for (i, (bi, bo)) in l.back_in.iter().zip(&l.back_out).enumerate() {
                    if bi.v != entry.backs[i] & bi.k || bo.v != back_out[i] & bo.k {
                        return Err(format!("back fill {i}"));
                    }
                }
                for (j, &o) in outputs.iter().enumerate() {
                    for (i, w) in l.outputs[j].iter().enumerate() {
                        if w.v != tv[o][i] & w.k {
                            return Err(format!("output {o} at block {}", at + i));
                        }
                    }
                }
                Ok(())
            };
            let why: Vec<String> = solved.leaves.iter().map(|l| holds(l)).filter_map(Result::err).collect();
            assert!(why.len() < solved.leaves.len(), "file {k}, blocks {range:?}: no leaf holds the truth ({} leaves, {} refuted): {why:?}", solved.leaves.len(), solved.refuted);
            chunks += 1;
        }
    }
    chunks
}

#[test]
fn rr_solve_is_sound() {
    assert!(solve_is_sound(rr(), &rr_files(6_000, 3), 4, 2, 1024) > 0);
}

#[test]
fn kotlin_solve_is_sound() {
    assert!(solve_is_sound(kotlin(), &kotlin_files(4_000, 3), 4, 2, 1024) > 0);
}

#[test]
fn json_solve_is_sound() {
    assert!(solve_is_sound(json(), &json_files(6_000, 6), 4, 2, 1024) > 0);
}

/// What the bytes alone leave of a chunk's start, over the corpora: per 1 KB chunk solved from
/// an unknown start, the leaves left (starts not refuted, split only where it mattered), whether
/// they are all clean (no refusal and no end component left unknown), and their distinct ends.
/// A report (`--ignored`); `RERAIL_KOTLIN_CORPUS` and `RERAIL_JSON_CORPUS` name the corpora.
fn report_solve(name: &str, program: &BlockProgram, files: &[Vec<u8>], look: usize, cap: usize, chunks_wanted: usize) {
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let mut all: Vec<(usize, usize)> = Vec::new();
    for (f, bytes) in files.iter().enumerate() {
        let n = bytes.len().div_ceil(64).max(1);
        all.extend((0..n).step_by(16).map(|at| (f, at)));
    }
    let step = all.len().div_ceil(chunks_wanted.max(1)).max(1);
    let (mut leaves, mut entries, mut widest, mut runs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut clean, mut over, mut one) = (0usize, 0usize, 0usize);
    let mut sampled = 0usize;
    let t0 = std::time::Instant::now();
    for &(f, at) in all.iter().step_by(step) {
        let file = File::new(program, &files[f]);
        let range = at..(at + 16).min(file.count());
        let t1 = std::time::Instant::now();
        let s = crate::solve::solve(&p, &comps, &file, range, look, cap);
        if std::env::var_os("RERAIL_FM_EACH").is_some() {
            eprintln!("   file {f} block {at}: {:.0} ms, {} entries, widest {}, {} runs, {} refuted, {} leaves, splits {:?}", t1.elapsed().as_secs_f64() * 1e3, s.entries, s.widest, s.runs, s.refuted, s.leaves.len(), s.splits);
        }
        sampled += 1;
        leaves.push(s.leaves.len());
        entries.push(s.entries);
        widest.push(s.widest);
        runs.push(s.runs);
        clean += usize::from(s.leaves.iter().all(|l| l.clean));
        over += usize::from(s.over);
        one += usize::from(s.leaves.len() == 1);
    }
    let pct = |v: &mut Vec<usize>, q: f64| -> usize {
        v.sort_unstable();
        v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0)
    };
    let row = |label: &str, v: &mut Vec<usize>| eprintln!("   {label:28} median {} p90 {} p99 {} max {}", pct(v, 0.5), pct(v, 0.9), pct(v, 0.99), pct(v, 1.0));
    eprintln!("== {name}: {sampled} chunks of 1 KB from an unknown start, {look} blocks of lookback (cap {cap}), {:.1} ms a chunk", t0.elapsed().as_secs_f64() * 1e3 / sampled.max(1) as f64);
    row("ends left (leaves):", &mut leaves);
    row("states at the chunk's start:", &mut entries);
    row("widest world set:", &mut widest);
    row("one-block runs:", &mut runs);
    eprintln!("   one end: {one}; all leaves clean: {clean}; over the cap: {over}");
}

#[test]
#[ignore = "a report over the corpora"]
fn report_the_solve() {
    let chunks = |v: &str| std::env::var(v).ok().and_then(|n| n.parse().ok());
    let look = chunks("RERAIL_FM_LOOK").unwrap_or(2);
    for (name, program, files, var, d) in [("json", json(), json_files(1 << 20, usize::MAX), "RERAIL_FM_JSON", 200), ("rr", rr(), rr_files(65_536, usize::MAX), "RERAIL_FM_RR", 100), ("kotlin", kotlin(), kotlin_files(65_536, usize::MAX), "RERAIL_FM_KOTLIN", 50)] {
        let n = chunks(var).unwrap_or(d);
        if n > 0 {
            report_solve(name, program, &files, look, 1024, n);
        }
    }
}


/// Diagnosis: with every fill, toggle and back fill fixed to the truth at the run's first block
/// (one block of lookback) and the rest unknown, each operation unknown at the chunk's last block
/// though its operands there are known: where the three values lose what the bytes decide.
fn report_sources(name: &str, program: &BlockProgram, files: &[Vec<u8>]) {
    use rerail_compiler::block::BlockOp;
    use std::collections::HashMap;
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let mut sources: HashMap<usize, usize> = HashMap::new();
    let mut never: HashMap<usize, usize> = HashMap::new();
    let mut n = 0;
    for bytes in files.iter().take(12) {
        if program.run(bytes).totals.iter().any(|&t| t != 0) {
            continue;
        }
        let file = File::new(program, bytes);
        let starts = serial_starts(&p, &file, bytes);
        for at in (16..file.count()).step_by(16) {
            let range = at..(at + 16).min(file.count());
            let from = at - 1;
            let mut start = p.unknown();
            let mut truth = starts[from].clone();
            truth.backs = starts[range.end - 1].backs.clone();
            for c in &comps {
                let long = c.in_place || matches!(c.at, crate::starts::Place::Back(_) | crate::starts::Place::Group(..)) || matches!(program.ops[c.op], BlockOp::Fill(..) | BlockOp::Toggle(_));
                if long {
                    let v = c.at.get(&truth);
                    start.set(c.at, match program.ops[c.op] {
                        BlockOp::Prev(_, k) => crate::solve::T { k: (1u64 << k) - 1, v },
                        BlockOp::Toggle(_) => crate::solve::T::known(v),
                        _ => crate::solve::T { k: 1, v },
                    });
                }
            }
            let (values, _) = p.run_t(&file, from..range.end, &start);
            n += 1;
            // Each operation's first block unknown, where its operands are known there and it
            // was known the block before (or it is the run's first block).
            for x in 0..program.ops.len() {
                let w = &values[x];
                if w.last().is_none_or(|w| w.full()) {
                    continue;
                }
                // The trailing unknown streak's first block.
                let j = w.iter().rposition(|w| w.full()).map_or(0, |k| k + 1);
                if j == 0 {
                    *never.entry(x).or_default() += 1;
                } else if program.ops[x].operands().iter().all(|&o| values[usize::from(o)].get(j).is_some_and(|w| w.full())) {
                    *sources.entry(x).or_default() += 1;
                }
            }
        }
    }
    let line = |x: usize, k: usize| {
        let ops: Vec<String> = program.ops[x].operands().iter().map(|&o| format!("#{o} {}", program.ops[usize::from(o)].text())).collect();
        let in_group = p.groups.iter().any(|g| g.members.iter().any(|&m| m == x));
        eprintln!("   {k:4}  #{x} {}{}   <- {}", program.ops[x].text(), if in_group { " [group]" } else { "" }, ops.join(" | "));
    };
    eprintln!("== {name}: {n} chunks, the true long-lived start; unknown at the chunk's end since the run's start");
    let mut v: Vec<_> = never.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (x, k) in v.iter().take(20) {
        line(*x, *k);
    }
    eprintln!("   ... lost on the way (operands known where the streak starts):");
    let mut v: Vec<_> = sources.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (x, k) in v.iter().take(20) {
        line(*x, *k);
    }
}

#[test]
#[ignore = "a report"]
fn report_the_sources() {
    report_sources("rr", rr(), &rr_files(65_536, usize::MAX));
    report_sources("kotlin", kotlin(), &kotlin_files(65_536, usize::MAX));
}

/// Where a chunk's leaves differ: for each start component, the chunks whose leaves' ends hold
/// it in more than one way (a value or unknown), with the number of ways at most.
fn report_differ(name: &str, program: &BlockProgram, files: &[Vec<u8>], chunks_wanted: usize) {
    use std::collections::{HashMap, HashSet};
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let mut all: Vec<(usize, usize)> = Vec::new();
    for (f, bytes) in files.iter().enumerate() {
        all.extend((16..bytes.len().div_ceil(64)).step_by(16).map(|at| (f, at)));
    }
    let step = all.len().div_ceil(chunks_wanted.max(1)).max(1);
    let mut differ: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut n = 0;
    for &(f, at) in all.iter().step_by(step) {
        let file = File::new(program, &files[f]);
        let s = crate::solve::solve(&p, &comps, &file, at..(at + 16).min(file.count()), 1, 256);
        n += 1;
        for (i, c) in comps.iter().enumerate() {
            if c.in_place {
                continue;
            }
            let ways: HashSet<(u64, u64)> = s.leaves.iter().map(|l| { let t = if matches!(c.at, crate::starts::Place::Back(_)) { l.back_in[match c.at { crate::starts::Place::Back(b) => b, _ => 0 }] } else { l.end.get(c.at) }; (t.k, t.v) }).collect();
            if ways.len() > 1 {
                let e = differ.entry(i).or_default();
                e.0 += 1;
                e.1 = e.1.max(ways.len());
            }
        }
    }
    let mut v: Vec<_> = differ.into_iter().collect();
    v.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    eprintln!("== {name}: {n} chunks; components the leaves' ends differ in");
    for (i, (k, w)) in v.iter().take(30) {
        let c = &comps[*i];
        let ops: Vec<String> = program.ops[c.op].operands().iter().map(|&o| format!("#{o} {}", program.ops[usize::from(o)].text())).collect();
        eprintln!("   {k:4} chunks, up to {w:3} ways  {:?} #{} {}   <- {}", c.at, c.op, program.ops[c.op].text(), ops.join(" | "));
    }
}

#[test]
#[ignore = "a report"]
fn report_the_differences() {
    report_differ("rr", rr(), &rr_files(65_536, usize::MAX), 12);
}

/// One chunk's leaves, each as its ends' long-lived components (0, 1, or ? unknown), with the
/// truth's, and the chunk's text.
#[test]
#[ignore = "a report"]
fn report_one_chunk() {
    use rerail_compiler::block::BlockOp;
    let program = rr();
    let files = rr_files(65_536, usize::MAX);
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let pick: usize = std::env::var("RERAIL_FM_FILE").ok().and_then(|n| n.parse().ok()).unwrap_or(3);
    let (f, bytes) = files.iter().enumerate().filter(|(_, b)| b.len() > 2_000).nth(pick).expect("a file");
    let file = File::new(program, bytes);
    let at = 16;
    let range = at..(at + 16).min(file.count());
    let s = crate::solve::solve(&p, &comps, &file, range.clone(), 1, 256);
    let starts = serial_starts(&p, &file, bytes);
    let long: Vec<usize> = (0..comps.len()).filter(|&i| !comps[i].in_place && matches!(program.ops[comps[i].op], BlockOp::Fill(..) | BlockOp::Toggle(_))).collect();
    eprintln!("== rr file {f}, blocks {range:?}: {} leaves, {} entries, widest {}", s.leaves.len(), s.entries, s.widest);
    eprintln!("   components: {}", long.iter().map(|&i| format!("#{}", comps[i].op)).collect::<Vec<_>>().join(" "));
    let show = |t: crate::solve::T| if t.k & 1 == 0 { '?' } else if t.v & 1 == 1 { '1' } else { '0' };
    let truth: String = long.iter().map(|&i| { let v = comps[i].at.get(&starts[range.end]); if v & 1 == 1 { '1' } else { '0' } }).collect();
    eprintln!("   truth end   {truth}");
    let mut rows: Vec<String> = s.leaves.iter().map(|l| format!("{}  entry {}  clean {}", long.iter().map(|&i| show(l.end.get(comps[i].at))).collect::<String>(), long.iter().map(|&i| show(l.entry.get(comps[i].at))).collect::<String>(), l.clean)).collect();
    rows.sort();
    for r in rows.iter().take(80) {
        eprintln!("   leaf end    {r}");
    }
    let text = String::from_utf8_lossy(&bytes[64 * (at - 1)..(64 * range.end).min(bytes.len())]);
    eprintln!("--- text (with the lookback block):\n{text}\n---");
}

#[test]
#[ignore = "a report"]
fn report_the_walks() {
    for (name, program) in [("rr", rr()), ("kotlin", kotlin()), ("json", json())] {
        let p = Prepared::new(program).expect("a program the chunks run");
        let comps = crate::starts::components(&p);
        eprintln!("== {name}: {} walks", program.walks.len());
        for w in &program.walks {
            eprintln!("   group {} hidden #{} {}", w.group, w.hidden, program.ops[usize::from(w.hidden)].text());
            for k in &w.kinds {
                let comp = comps.iter().position(|c| c.op == usize::from(k.fill));
                eprintln!("     kind: opener #{} closer #{} open #{} fill #{} {} (component {comp:?}) bytes {} opens {} outside {} empty {:?} or {:?} after {} resumes {}", k.opener, k.closer, k.open, k.fill, program.ops[usize::from(k.fill)].text(), k.bytes, k.opens, k.outside, k.empty, k.or, k.after, k.resumes);
            }
        }
    }
}

#[test]
#[ignore = "a report: Kotlin from the true state two blocks before a chunk, the short-lived state unknown"]
fn report_kotlin_sources() {
    let program = kotlin();
    let files = kotlin_files(65_536, usize::MAX);
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let shape = crate::solve::shape(program, &comps);
    let n = |v: &str, d: usize| std::env::var(v).ok().and_then(|n| n.parse().ok()).unwrap_or(d);
    let (f, at, look) = (n("RERAIL_FM_FILE", 364), n("RERAIL_FM_AT", 176), n("RERAIL_FM_LOOK", 2));
    let file = File::new(program, &files[f]);
    let truth = serial_starts(&p, &file, &files[f]);
    let t = &truth[at - look];
    let mut s = crate::solve::TStart { ops: t.ops.iter().map(|&v| crate::solve::T::known(v)).collect(), groups: t.groups.iter().map(|g| g.iter().map(|&v| crate::solve::T::known(v)).collect()).collect(), backs: t.backs.iter().map(|&v| crate::solve::T::known(v)).collect() };
    // RERAIL_FM_KEEP=group keeps the group's members known; RERAIL_FM_ONLY=i makes only
    // component i unknown.
    let keep_group = std::env::var("RERAIL_FM_KEEP").is_ok_and(|v| v == "group");
    let only: Option<usize> = std::env::var("RERAIL_FM_ONLY").ok().and_then(|v| v.parse().ok());
    for (i, c) in comps.iter().enumerate() {
        let short = !c.in_place && !shape.long[i] && !shape.lazy[i] && !matches!(c.at, crate::starts::Place::Back(_));
        let group = matches!(c.at, crate::starts::Place::Group(..));
        if short && only.map_or(!(keep_group && group), |o| o == i) {
            s.set(c.at, crate::solve::T::X);
        }
    }
    if std::env::var_os("RERAIL_FM_EACH_GROUP").is_some() {
        // Each short-lived group component alone: the unknown it leaves at the block's end.
        let base = s.clone();
        let t0 = crate::solve::TStart { ops: t.ops.iter().map(|&v| crate::solve::T::known(v)).collect(), groups: t.groups.iter().map(|g| g.iter().map(|&v| crate::solve::T::known(v)).collect()).collect(), backs: t.backs.iter().map(|&v| crate::solve::T::known(v)).collect() };
        let _ = base;
        for (i, c) in comps.iter().enumerate() {
            if c.in_place || shape.long[i] || shape.lazy[i] || !matches!(c.at, crate::starts::Place::Group(..)) {
                continue;
            }
            let mut one = t0.clone();
            one.set(c.at, crate::solve::T::X);
            let (_, end) = p.run_t(&file, at - look..at - look + 1, &one);
            let unknown = comps.iter().filter(|c| !c.in_place && !matches!(c.at, crate::starts::Place::Back(_)) && { let w = crate::solve::width(program, c); end.get(c.at).k & w != w }).count();
            if unknown > 0 {
                eprintln!("   alone: component {i} #{} {} leaves {unknown} unknown", c.op, program.ops[c.op].text());
            }
        }
    }
    for b in at - look..at {
        let (values, end) = p.run_t(&file, b..b + 1, &s);
        let unknown: Vec<usize> = comps.iter().filter(|c| !c.in_place && !matches!(c.at, crate::starts::Place::Back(_)) && { let w = crate::solve::width(program, c); end.get(c.at).k & w != w }).map(|c| c.op).collect();
        let sources: Vec<String> = (0..program.ops.len()).filter(|&x| !values[x][0].full() && program.ops[x].operands().iter().all(|&o| values[usize::from(o)][0].full())).map(|x| format!("#{x} {}", program.ops[x].text())).collect();
        eprintln!("== block {b}: {} unknown at its end: {:?}", unknown.len(), unknown);
        eprintln!("   sources: {}", sources.join("; "));
        s = end;
        s.backs.iter_mut().zip(&truth[b + 1].backs).for_each(|(x, &v)| *x = crate::solve::T::known(v));
    }
}

#[test]
#[ignore = "a report: the operations under Kotlin's values RERAIL_FM_OPS, three deep"]
fn report_kotlin_ops() {
    let program = kotlin();
    let ops: Vec<usize> = std::env::var("RERAIL_FM_OPS").unwrap_or_default().split(',').filter_map(|x| x.trim().parse().ok()).collect();
    fn show(program: &BlockProgram, x: usize, depth: usize, seen: &mut Vec<usize>) {
        let pad = "  ".repeat(depth);
        eprintln!("   {pad}#{x} {}", program.ops[x].text());
        if depth >= 4 || seen.contains(&x) {
            return;
        }
        seen.push(x);
        for o in program.ops[x].operands() {
            show(program, usize::from(o), depth + 1, seen);
        }
    }
    for x in ops {
        show(program, x, 0, &mut Vec::new());
    }
}

#[test]
#[ignore = "a report: Kotlin's start, its groups' carries and the sets apart"]
fn report_the_groups() {
    let program = kotlin();
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    for (g, group) in program.rounds.iter().enumerate() {
        eprintln!("== group {g}: {} carries", group.len());
        for &(c, x) in group {
            eprintln!("   carry #{c} {} next #{x} {}", program.ops[usize::from(c)].text(), program.ops[usize::from(x)].text());
        }
    }
    for (i, c) in comps.iter().enumerate().filter(|(_, c)| !c.in_place) {
        eprintln!("   component {i} {:?} #{} {}", c.at, c.op, program.ops[c.op].text());
    }
    for set in &program.apart {
        eprintln!("   apart {:?}", set.iter().map(|&v| (v, comps.iter().position(|c| c.op == usize::from(v)))).collect::<Vec<_>>());
    }
}

/// Each file in `chunk`-block chunks, each solved from an unknown start, then joined. Every
/// output bit the join decides is the serial run's, and a chunk not decided (an output bit
/// unknown) is counted: the ones a device runs again. Returns (chunks, run again).
fn join_is_the_serial_run(program: &BlockProgram, files: &[Vec<u8>], chunk: usize, cap: usize) -> (usize, usize) {
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let outputs: Vec<usize> = program.outputs.iter().map(|&o| usize::from(o)).collect();
    let (mut chunks, mut again) = (0, 0);
    for (k, bytes) in files.iter().enumerate() {
        if program.run(bytes).totals.iter().any(|&t| t != 0) {
            continue;
        }
        let file = File::new(program, bytes);
        let (want, _) = program.rounds_values(&[bytes]);
        let ranges: Vec<std::ops::Range<usize>> = (0..file.count()).step_by(chunk).map(|at| at..(at + chunk).min(file.count())).collect();
        let solved: Vec<crate::solve::Solved> = ranges.iter().map(|r| crate::solve::solve(&p, &comps, &file, r.clone(), 2, cap)).collect();
        let alive = crate::join::join(&p, &solved);
        for (c, r) in ranges.iter().enumerate() {
            chunks += 1;
            assert!(!alive[c].is_empty(), "file {k}, chunk {r:?}: the join kept no leaf");
            let got = crate::join::outputs(&solved[c], &alive[c]).expect("a leaf");
            let mut decided = true;
            for (j, &o) in outputs.iter().enumerate() {
                for (i, w) in got[j].iter().enumerate() {
                    assert_eq!(w.v, want[o][r.start + i] & w.k, "file {k}, chunk {r:?}: output {o} at block {}", r.start + i);
                    decided &= w.full();
                }
            }
            again += usize::from(!decided);
            if !decided && std::env::var("RERAIL_FM_WHY").is_ok() {
                eprintln!("   undecided: file {k} chunk {r:?}: {} kept of {} leaves, {} entries, splits {:?}, spent {}", alive[c].len(), solved[c].leaves.len(), solved[c].entries, solved[c].splits, solved[c].spent);
                for (r, e, o, b) in &solved[c].open_at_budget {
                    eprintln!("     open at the budget: end {e:?} outputs {o:?} back {b}; sources:");
                    for &x in r {
                        let ops: Vec<String> = program.ops[x].operands().iter().map(|&q| format!("#{q} {}", program.ops[usize::from(q)].text())).collect();
                        eprintln!("       #{x} {}   <- {}", program.ops[x].text(), ops.join(" | "));
                    }
                }
            }
        }
    }
    (chunks, again)
}

#[test]
fn rr_join_is_the_serial_run() {
    let (n, again) = join_is_the_serial_run(rr(), &rr_files(6_000, 3), 4, 1024);
    eprintln!("rr: {again} of {n} chunks run again");
}

/// Kotlin's stack (decision 0621): a chunk's entry fixes the depth only; the kind of a level
/// it pops below its own pushes is split on where it decides an output, and the join checks it.
#[test]
fn kotlin_join_is_the_serial_run() {
    let (n, again) = join_is_the_serial_run(kotlin(), &kotlin_files(4_000, 3), 4, 1024);
    eprintln!("kotlin: {again} of {n} chunks run again");
}

#[test]
#[ignore = "a report: Kotlin's join over the files of at most RERAIL_FM_MAX bytes, the first RERAIL_FM_COUNT"]
fn report_kotlin_join() {
    let n = |v: &str, d: usize| std::env::var(v).ok().and_then(|n| n.parse().ok()).unwrap_or(d);
    let t0 = std::time::Instant::now();
    let (c, again) = join_is_the_serial_run(kotlin(), &kotlin_files(n("RERAIL_FM_MAX", 8_000), n("RERAIL_FM_COUNT", 6)), 16, 1024);
    eprintln!("== kotlin: {again} of {c} chunks of 1 KB run again, {:.1} s", t0.elapsed().as_secs_f64());
}

#[test]
fn json_join_is_the_serial_run() {
    let (n, again) = join_is_the_serial_run(json(), &json_files(6_000, 6), 4, 1024);
    eprintln!("json: {again} of {n} chunks run again");
}

/// The join over the corpora, in 1 KB chunks: the chunks it leaves undecided (run again).
#[test]
#[ignore = "a report over the corpora"]
fn report_the_join() {
    let files = |v: Vec<Vec<u8>>, n: usize| v.into_iter().take(n).collect::<Vec<_>>();
    let n = |v: &str, d: usize| std::env::var(v).ok().and_then(|n| n.parse().ok()).unwrap_or(d);
    for (name, program, fs) in [
        ("json", json(), files(json_files(1 << 20, usize::MAX), n("RERAIL_FM_JSON", 20))),
        ("rr", rr(), files(rr_files(65_536, usize::MAX), n("RERAIL_FM_RR", 40))),
        ("kotlin", kotlin(), files(kotlin_files(65_536, usize::MAX), n("RERAIL_FM_KOTLIN", 4))),
    ] {
        if fs.is_empty() {
            continue;
        }
        let t0 = std::time::Instant::now();
        let (c, again) = join_is_the_serial_run(program, &fs, 16, 1024);
        eprintln!("== {name}: {} files, {c} chunks of 1 KB, {again} run again ({:.2}%), {:.1} s", fs.len(), 100.0 * again as f64 / c.max(1) as f64, t0.elapsed().as_secs_f64());
    }
}

/// After the join, the output bits left unknown: by output, by block of the chunk, and the
/// number of leaves kept.
#[test]
#[ignore = "a report"]
fn report_join_unknowns() {
    use std::collections::HashMap;
    let program = rr();
    let files: Vec<Vec<u8>> = rr_files(65_536, usize::MAX).into_iter().take(6).collect();
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let outputs: Vec<usize> = program.outputs.iter().map(|&o| usize::from(o)).collect();
    let (mut by_out, mut by_block, mut kept): (HashMap<usize, usize>, HashMap<usize, usize>, HashMap<usize, usize>) = Default::default();
    let mut bits_by_out: HashMap<usize, u64> = HashMap::new();
    for bytes in &files {
        let file = File::new(program, bytes);
        let ranges: Vec<std::ops::Range<usize>> = (0..file.count()).step_by(16).map(|at| at..(at + 16).min(file.count())).collect();
        let solved: Vec<crate::solve::Solved> = ranges.iter().map(|r| crate::solve::solve(&p, &comps, &file, r.clone(), 1, 256)).collect();
        let alive = crate::join::join(&p, &solved);
        for (c, _) in ranges.iter().enumerate() {
            *kept.entry(alive[c].len()).or_default() += 1;
            let got = crate::join::outputs(&solved[c], &alive[c]).expect("a leaf");
            for (j, &o) in outputs.iter().enumerate() {
                for (i, w) in got[j].iter().enumerate() {
                    if !w.full() {
                        *by_out.entry(o).or_default() += 1;
                        *by_block.entry(i).or_default() += 1;
                        *bits_by_out.entry(o).or_default() |= !w.k;
                    }
                }
            }
        }
    }
    let sorted = |m: HashMap<usize, usize>| { let mut v: Vec<_> = m.into_iter().collect(); v.sort(); v };
    eprintln!("== rr join: leaves kept per chunk {:?}", sorted(kept));
    eprintln!("   unknown words by block of the chunk {:?}", sorted(by_block));
    for (o, n) in sorted(by_out) {
        eprintln!("   output #{o} {}: {n} words, unknown bits {:#018x}", program.ops[o].text(), bits_by_out[&o]);
    }
}

/// One rr file's first chunks after the join: each kept leaf's entry and end (every
/// non-in-place component, 0/1/? by bit 0 of its value, `*` a wider value partly known), its back
/// carries, and its outputs' known share; with the truth.
#[test]
#[ignore = "a report"]
fn report_join_detail() {
    let program = rr();
    let files = rr_files(65_536, usize::MAX);
    let pick: usize = std::env::var("RERAIL_FM_FILE").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let bytes = &files[pick];
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let file = File::new(program, bytes);
    let starts = serial_starts(&p, &file, bytes);
    let ranges: Vec<std::ops::Range<usize>> = (0..file.count()).step_by(16).map(|at| at..(at + 16).min(file.count())).collect();
    let solved: Vec<crate::solve::Solved> = ranges.iter().map(|r| crate::solve::solve(&p, &comps, &file, r.clone(), 1, 256)).collect();
    let alive = crate::join::join(&p, &solved);
    let fwd: Vec<usize> = (0..comps.len()).filter(|&i| !comps[i].in_place && !matches!(comps[i].at, crate::starts::Place::Back(_))).collect();
    let show = |s: &crate::solve::TStart| -> String { fwd.iter().map(|&i| { let t = s.get(comps[i].at); let w = crate::solve::width(program, &comps[i]); if t.k & w == w { if t.v != 0 { '1' } else { '0' } } else if t.k & w == 0 { '?' } else { '*' } }).collect() };
    let truth = |at: usize| -> String { fwd.iter().map(|&i| if comps[i].at.get(&starts[at]) != 0 { '1' } else { '0' }).collect() };
    eprintln!("== rr file {pick}: {} blocks, {} chunks; components {}", file.count(), ranges.len(), fwd.iter().map(|&i| format!("#{}", comps[i].op)).collect::<Vec<_>>().join(" "));
    let undecided = |c: usize| crate::join::outputs(&solved[c], &alive[c]).is_none_or(|o| o.iter().flatten().any(|w| !w.full()));
    for (c, r) in ranges.iter().enumerate().filter(|(c, _)| undecided(*c)).take(3) {
        eprintln!("-- chunk {c} {r:?}: {} leaves, {} kept; truth entry {}", solved[c].leaves.len(), alive[c].len(), truth(r.start));
        for &i in &alive[c] {
            let l = &solved[c].leaves[i];
            let known: usize = l.outputs.iter().flatten().map(|w| w.k.count_ones() as usize).sum();
            let all: usize = l.outputs.iter().flatten().count() * 64;
            eprintln!("   entry {}\n   end   {}  back in {:?} out {:?}  outputs known {known}/{all}", show(&l.entry), show(&l.end), l.back_in.iter().map(|t| (t.k & 1, t.v & 1)).collect::<Vec<_>>(), l.back_out.iter().map(|t| (t.k & 1, t.v & 1)).collect::<Vec<_>>());
        }
        eprintln!("   truth end {}", truth(r.end));
    }
}

/// For the join's undecided rr chunks: each kept leaf's entry components left unknown that are in
/// the cone of its unknown outputs, by kind.
#[test]
#[ignore = "a report"]
fn report_undecided_causes() {
    use std::collections::HashMap;
    let program = rr();
    let files: Vec<Vec<u8>> = rr_files(65_536, usize::MAX).into_iter().skip(3).take(2).collect();
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let cones = crate::solve::cones(program, &comps);
    let outputs: Vec<usize> = program.outputs.iter().map(|&o| usize::from(o)).collect();
    let mut causes: HashMap<usize, usize> = HashMap::new();
    let mut leaves = 0;
    for bytes in &files {
        let file = File::new(program, bytes);
        let ranges: Vec<std::ops::Range<usize>> = (0..file.count()).step_by(16).map(|at| at..(at + 16).min(file.count())).collect();
        let solved: Vec<crate::solve::Solved> = ranges.iter().map(|r| crate::solve::solve(&p, &comps, &file, r.clone(), 1, 256)).collect();
        let alive = crate::join::join(&p, &solved);
        for (c, _) in ranges.iter().enumerate() {
            for &i in &alive[c] {
                let l = &solved[c].leaves[i];
                let unknown_outs: Vec<usize> = outputs.iter().enumerate().filter(|(j, _)| l.outputs[*j].iter().any(|w| !w.full())).map(|(_, &o)| o).collect();
                if unknown_outs.is_empty() {
                    continue;
                }
                leaves += 1;
                for (k, comp) in comps.iter().enumerate() {
                    let w = crate::solve::width(program, comp);
                    if !comp.in_place && l.entry.get(comp.at).k & w != w && unknown_outs.iter().any(|&o| cones[o][k]) {
                        *causes.entry(k).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut v: Vec<_> = causes.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("== rr: {leaves} kept leaves with an output unknown; their unknown entry components in its cone");
    for (k, n) in v.iter().take(25) {
        eprintln!("   {n:4}  {:?} #{} {}", comps[*k].at, comps[*k].op, program.ops[comps[*k].op].text());
    }
}

/// The entries the run to one rr chunk leaves: each one's unknown components.
#[test]
#[ignore = "a report"]
fn report_kotlin_entries() {
    let program = kotlin();
    let files = kotlin_files(65_536, usize::MAX);
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let n = |v: &str, d: usize| std::env::var(v).ok().and_then(|n| n.parse().ok()).unwrap_or(d);
    // `report_solve`'s sample `RERAIL_FM_SAMPLE` of `RERAIL_FM_KOTLIN` (default the second of 4).
    let mut all: Vec<(usize, usize)> = Vec::new();
    for (f, bytes) in files.iter().enumerate() {
        all.extend((0..bytes.len().div_ceil(64).max(1)).step_by(16).map(|at| (f, at)));
    }
    let step = all.len().div_ceil(n("RERAIL_FM_KOTLIN", 4).max(1)).max(1);
    let (f, at) = all[step * n("RERAIL_FM_SAMPLE", 1)];
    let file = File::new(program, &files[f]);
    let s = crate::solve::entries(&p, &comps, &file, at..at + 16, n("RERAIL_FM_LOOK", 2), 1024);
    let truth = serial_starts(&p, &file, &files[f]);
    eprintln!("== kotlin file {f} chunk {at}: {} entries; refuted {} runs {}", s.entries, s.refuted, s.runs);
    eprintln!("   bounds: {:?}", crate::solve::bounds(&p, &comps).iter().map(|&(f, r)| (comps[f].op, comps[r].op)).collect::<Vec<_>>());
    let stacks = crate::solve::stacks(program, &comps);
    let walks = crate::solve::walks(program, &comps);
    let show = |e: &crate::solve::TStart| {
        let bit = |c: usize| { let t = e.get(comps[c].at); if t.k & 1 == 0 { '?' } else if t.v & 1 == 1 { '1' } else { '0' } };
        let depth: Vec<String> = stacks.iter().map(|st| st.levels.iter().map(|&l| bit(l)).collect()).collect();
        let sets: Vec<String> = walks.fills.iter().map(|fs| fs.iter().map(|&f| bit(f)).collect()).collect();
        let unknown = comps.iter().filter(|c| !c.in_place && !matches!(c.at, crate::starts::Place::Back(_))).filter(|c| { let w = crate::solve::width(program, c); e.get(c.at).k & w != w }).count();
        format!("levels {depth:?} sets {sets:?} {unknown} unknown")
    };
    let t = &truth[at];
    let tt = crate::solve::TStart { ops: t.ops.iter().map(|&v| crate::solve::T::known(v)).collect(), groups: t.groups.iter().map(|g| g.iter().map(|&v| crate::solve::T::known(v)).collect()).collect(), backs: t.backs.iter().map(|&v| crate::solve::T::known(v)).collect() };
    eprintln!("   truth: {}", show(&tt));
    let mut lines: Vec<String> = s.entry_states.iter().map(|e| show(e)).collect();
    lines.sort();
    let mut counted: Vec<(usize, String)> = Vec::new();
    for l in lines {
        match counted.last_mut() {
            Some((c, m)) if *m == l => *c += 1,
            _ => counted.push((1, l)),
        }
    }
    for (c, l) in counted {
        eprintln!("   {c:4} {l}");
    }
    // The entry with the fewest unknown, and what is unknown in it.
    let unknown = |e: &crate::solve::TStart| comps.iter().filter(|c| !c.in_place && !matches!(c.at, crate::starts::Place::Back(_))).filter(|c| { let w = crate::solve::width(program, c); e.get(c.at).k & w != w }).map(|c| format!("#{} {}", c.op, program.ops[c.op].text())).collect::<Vec<String>>();
    if let Some(e) = s.entry_states.iter().min_by_key(|e| unknown(e).len()) {
        eprintln!("   fewest: {}", show(e));
        for u in unknown(e) {
            eprintln!("     {u}");
        }
    }
    let agrees: Vec<&crate::solve::TStart> = s.entry_states.iter().filter(|e| crate::join::agree_state(e, &tt)).collect();
    eprintln!("   {} entries agree with the truth", agrees.len());
    if let Some(e) = agrees.iter().min_by_key(|e| unknown(e).len()) {
        eprintln!("   true: {}", show(e));
        for u in unknown(e) {
            eprintln!("     {u}");
        }
    }
}

#[test]
#[ignore = "a report"]
fn report_entries() {
    let program = rr();
    let files = rr_files(65_536, usize::MAX);
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let file = File::new(program, &files[0]);
    let look: usize = std::env::var("RERAIL_FM_LOOK").ok().and_then(|n| n.parse().ok()).unwrap_or(1);
    let s = crate::solve::solve(&p, &comps, &file, 4..8, look, 256);
    eprintln!("== rr file 0 chunk 4..8, look {look}: {} entries; refuted {} runs {}", s.entries, s.refuted, s.runs);
    for e in &s.entry_states {
        let unknown: Vec<String> = comps.iter().filter(|c| !c.in_place && !matches!(c.at, crate::starts::Place::Back(_))).filter(|c| { let w = crate::solve::width(program, c); e.get(c.at).k & w != w }).map(|c| format!("#{} {}", c.op, program.ops[c.op].text())).collect();
        eprintln!("   {} unknown: {}", unknown.len(), unknown.join(", "));
    }
}

/// What a ranked join did over files (`join_ranked_is_the_serial_run`).
#[derive(Debug, Default)]
struct Ranked {
    chunks: usize,
    /// Chunks whose outputs the join left unknown somewhere.
    undecided: usize,
    /// Chunks the join settled on a start that was not their best state.
    reextracted: usize,
    /// Chunks whose start was dropped: run again from their now known start.
    rerun: usize,
    /// Per chunk, the states it kept.
    states: Vec<usize>,
    bytes: usize,
}

/// Each file in `chunk`-block chunks, each solving the entries of its best `m` states by
/// priors measured on `train`, then joined with the dropped run again. Every output bit decided
/// is the serial run's.
fn join_ranked_is_the_serial_run(program: &BlockProgram, train: &[Vec<u8>], files: &[Vec<u8>], chunk: usize, cap: usize, m: usize) -> Ranked {
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let priors = crate::rank::Priors::measure(&p, &comps, train);
    let outputs: Vec<usize> = program.outputs.iter().map(|&o| usize::from(o)).collect();
    let mut out = Ranked::default();
    for (k, bytes) in files.iter().enumerate() {
        if program.run(bytes).totals.iter().any(|&t| t != 0) {
            continue;
        }
        out.bytes += bytes.len();
        let file = File::new(program, bytes);
        let (want, _) = program.rounds_values(&[bytes]);
        let ranges: Vec<std::ops::Range<usize>> = (0..file.count()).step_by(chunk).map(|at| at..(at + chunk).min(file.count())).collect();
        let mut solved: Vec<crate::solve::Solved> = ranges.iter().map(|r| crate::solve::solve_with(&p, &comps, &file, r.clone(), 2, cap, crate::solve::Entries::Ranked(&priors, m))).collect();
        for s in &solved {
            let mut ks: Vec<usize> = s.leaves.iter().map(|l| l.rank).collect();
            ks.sort_unstable();
            ks.dedup();
            out.states.push(ks.len());
        }
        let (alive, rerun) = crate::join::join_with(&p, &mut solved, &mut |c, ends| Some(crate::solve::solve_with(&p, &comps, &file, ranges[c].clone(), 2, cap, crate::solve::Entries::Given(ends))));
        out.rerun += rerun.len();
        for (c, r) in ranges.iter().enumerate() {
            out.chunks += 1;
            assert!(!alive[c].is_empty(), "file {k}, chunk {r:?}: the join kept no leaf");
            if !rerun.contains(&c) && alive[c].iter().all(|&i| solved[c].leaves[i].rank > 0) {
                out.reextracted += 1;
            }
            let got = crate::join::outputs(&solved[c], &alive[c]).expect("a leaf");
            let mut decided = true;
            for (j, &o) in outputs.iter().enumerate() {
                for (i, w) in got[j].iter().enumerate() {
                    assert_eq!(w.v, want[o][r.start + i] & w.k, "file {k}, chunk {r:?}: output {o} at block {}", r.start + i);
                    decided &= w.full();
                }
            }
            out.undecided += usize::from(!decided);
        }
    }
    out
}

/// The design page's case (§5): a 3 KB JSON array of numbers, and the same bytes as one string.
/// No refusal ends a middle chunk's ambiguity; the priors rank "outside a string" first, so the
/// array's chunks take their best start and the string's run again. Both are the serial run.
#[test]
fn json_ranked_numbers_and_their_string() {
    let mut numbers = String::new();
    let mut i = 0;
    while numbers.len() < 3_000 {
        numbers.push_str(&format!("{i}, "));
        i += 7;
    }
    let array = format!("[{numbers}0]\n").into_bytes();
    let string = format!("[\"{numbers}0\"]\n").into_bytes();
    let train = json_files(1 << 20, 20);
    let a = join_ranked_is_the_serial_run(json(), &train, std::slice::from_ref(&array), 4, 1024, 1);
    let s = join_ranked_is_the_serial_run(json(), &train, std::slice::from_ref(&string), 4, 1024, 1);
    eprintln!("json, M = 1: the array {a:?}; the string {s:?}");
    assert_eq!(a.rerun, 0, "the array's chunks start outside a string, the best state");
    assert!(s.rerun > 0, "the string's middle chunks start inside it, ranked below");
}

#[test]
fn rr_ranked_join_is_the_serial_run() {
    let all = rr_files(6_000, 30);
    let r = join_ranked_is_the_serial_run(rr(), &all[3..], &all[..3], 4, 1024, 2);
    eprintln!("rr, M = 2: {r:?}");
}

#[test]
#[ignore = "a report: the ranked join over the corpora at M = 2, 4, 8 (RERAIL_FM_MS; RERAIL_FM_RR, RERAIL_FM_JSON, RERAIL_FM_KOTLIN files; priors from the next 100)"]
fn report_the_ranked_join() {
    let n = |v: &str, d: usize| std::env::var(v).ok().and_then(|n| n.parse().ok()).unwrap_or(d);
    for (name, program, all, count) in [
        ("json", json(), json_files(1 << 20, usize::MAX), n("RERAIL_FM_JSON", 20)),
        ("rr", rr(), rr_files(65_536, usize::MAX), n("RERAIL_FM_RR", 12)),
        ("kotlin", kotlin(), kotlin_files(n("RERAIL_FM_MAX", 65_536), usize::MAX), n("RERAIL_FM_KOTLIN", 0)),
    ] {
        if count == 0 || all.len() <= count {
            continue;
        }
        let (files, train) = all.split_at(count);
        let train = &train[..train.len().min(100)];
        let ms: Vec<usize> = std::env::var("RERAIL_FM_MS").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![2, 4, 8]);
        for &m in &ms {
            let t0 = std::time::Instant::now();
            let r = join_ranked_is_the_serial_run(program, train, files, 16, 1024, m);
            let mut st = r.states.clone();
            st.sort_unstable();
            let q = |f: f64| st.get(((st.len() as f64 - 1.0) * f).round() as usize).copied().unwrap_or(0);
            let mb = r.bytes as f64 / 1e6;
            eprintln!(
                "== {name}, M = {m}: {} chunks of 1 KB; states kept median {} p99 {}; {} re-extracted, {} run again ({:.2} / MB), {} undecided; {:.1} s",
                r.chunks, q(0.5), q(0.99), r.reextracted, r.rerun, r.rerun as f64 / mb.max(1e-9), r.undecided, t0.elapsed().as_secs_f64()
            );
        }
    }
}

#[test]
#[ignore = "a report: rr's chunk states by score against the true state (RERAIL_FM_FILE)"]
fn report_rr_ranks() {
    let program = rr();
    let all = rr_files(65_536, usize::MAX);
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let priors = crate::rank::Priors::measure(&p, &comps, &all[12..all.len().min(112)]);
    let f = std::env::var("RERAIL_FM_FILE").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let file = File::new(program, &all[f]);
    let truth = serial_starts(&p, &file, &all[f]);
    for at in (16..file.count()).step_by(16).take(8) {
        let s = crate::solve::entries(&p, &comps, &file, at..(at + 16).min(file.count()), 2, 1024);
        let mut keyed: Vec<(Vec<u8>, f64)> = Vec::new();
        for e in &s.entry_states {
            let (k, sc) = (priors.keys.of(&comps, e), priors.score(&p, &comps, &file, at..(at + 16).min(file.count()), e));
            match keyed.iter_mut().find(|x| x.0 == k) {
                Some(x) => x.1 = x.1.max(sc),
                None => keyed.push((k, sc)),
            }
        }
        keyed.sort_by(|a, b| b.1.total_cmp(&a.1));
        let t = priors.keys.of_start(&comps, &truth[at]);
        let rank = keyed.iter().position(|x| x.0 == t);
        eprintln!("== block {at}: truth {t:?} rank {rank:?}; {}", keyed.iter().map(|(k, s)| format!("{k:?} {s:.1}")).collect::<Vec<_>>().join(", "));
    }
}
