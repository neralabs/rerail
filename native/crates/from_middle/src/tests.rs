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
/// only checks the answer: the true start lies in a leaf the solve kept, and in every leaf it
/// lies in, each bit the leaf calls known (its outputs, its end) is the true run's.
fn solve_is_sound(program: &BlockProgram, files: &[Vec<u8>], chunk: usize, look: usize, cap: usize) -> usize {
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
        for at in (0..file.count()).step_by(chunk) {
            let range = at..(at + chunk).min(file.count());
            let solved = crate::solve::solve(&p, &comps, &file, range.clone(), look, cap);
            // The truth, to check against only: at the run's first block, and over the chunk.
            let mut truth = starts[solved.from].clone();
            truth.backs = starts[range.end - 1].backs.clone();
            let mut entry = starts[at].clone();
            entry.backs = truth.backs.clone();
            let (tv, tend) = p.run(&file, range.clone(), &entry);
            let holding: Vec<&crate::solve::Leaf> = solved.leaves.iter().filter(|l| l.starts.iter().any(|fixed| comps.iter().zip(fixed).all(|(c, f)| f.is_none_or(|v| v == c.at.get(&truth))))).collect();
            assert!(!holding.is_empty(), "file {k}, blocks {range:?}: the true start was refuted ({} leaves, {} refuted)", solved.leaves.len(), solved.refuted);
            for leaf in holding {
                for (j, &o) in outputs.iter().enumerate() {
                    for (i, w) in leaf.outputs[j].iter().enumerate() {
                        assert_eq!(w.v, tv[o][i] & w.k, "file {k}, blocks {range:?}: output {o} at block {} known wrong", at + i);
                    }
                }
                for c in comps.iter().filter(|c| !c.in_place) {
                    let w = leaf.end.get(c.at);
                    assert_eq!(w.v, c.at.get(&tend) & w.k, "file {k}, blocks {range:?}: end of op {} known wrong", c.op);
                    if !matches!(c.at, crate::starts::Place::Back(_)) {
                        let w = leaf.entry.get(c.at);
                        assert_eq!(w.v, c.at.get(&entry) & w.k, "file {k}, blocks {range:?}: entry of op {} known wrong", c.op);
                    }
                }
            }
            chunks += 1;
        }
    }
    chunks
}

#[test]
fn rr_solve_is_sound() {
    assert!(solve_is_sound(rr(), &rr_files(6_000, 6), 4, 1, 64) > 0);
}

#[test]
fn kotlin_solve_is_sound() {
    assert!(solve_is_sound(kotlin(), &kotlin_files(4_000, 3), 4, 1, 64) > 0);
}

#[test]
fn json_solve_is_sound() {
    assert!(solve_is_sound(json(), &json_files(6_000, 6), 4, 1, 64) > 0);
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
    let (mut leaves, mut ends, mut runs) = (Vec::new(), Vec::new(), Vec::new());
    let (mut clean, mut over, mut one) = (0usize, 0usize, 0usize);
    let mut sampled = 0usize;
    for &(f, at) in all.iter().step_by(step) {
        let file = File::new(program, &files[f]);
        let range = at..(at + 16).min(file.count());
        let s = crate::solve::solve(&p, &comps, &file, range, look, cap);
        sampled += 1;
        leaves.push(s.leaves.len());
        runs.push(s.runs);
        clean += usize::from(s.leaves.iter().all(|l| l.clean));
        over += usize::from(s.over);
        one += usize::from(s.leaves.len() == 1);
        ends.push(s.leaves.iter().map(|l| l.starts.len()).sum());
    }
    let pct = |v: &mut Vec<usize>, q: f64| -> usize {
        v.sort_unstable();
        v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0)
    };
    eprintln!("== {name}: {sampled} chunks of 1 KB from an unknown start, {look} blocks of lookback (cap {cap})");
    eprintln!("   distinct ends (leaves): median {} p90 {} p99 {} max {}", pct(&mut leaves, 0.5), pct(&mut leaves, 0.9), pct(&mut leaves, 0.99), pct(&mut leaves, 1.0));
    eprintln!("   partial starts kept: median {} p90 {} p99 {} max {}; chunks with one end: {one}", pct(&mut ends, 0.5), pct(&mut ends, 0.9), pct(&mut ends, 0.99), pct(&mut ends, 1.0));
    eprintln!("   runs:          median {} p99 {} max {}", pct(&mut runs, 0.5), pct(&mut runs, 0.99), pct(&mut runs, 1.0));
    eprintln!("   all leaves clean: {clean}; hit the cap: {over}");
}

#[test]
#[ignore = "a report over the corpora"]
fn report_the_solve() {
    for look in [1, 2] {
        report_solve("rr", rr(), &rr_files(65_536, usize::MAX), look, 256, 300);
        report_solve("kotlin", kotlin(), &kotlin_files(65_536, usize::MAX), look, 256, 150);
        report_solve("json", json(), &json_files(1 << 20, usize::MAX), look, 256, 300);
    }
}

/// What stays unknown in a chunk solved from an unknown start with no split: each state
/// component open at the end, each refusal open, with the operation and its operands.
fn report_open(name: &str, program: &BlockProgram, files: &[Vec<u8>]) {
    use std::collections::HashMap;
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let (mut ends, mut refusals, mut split): (HashMap<usize, usize>, HashMap<usize, usize>, HashMap<usize, usize>) = (HashMap::new(), HashMap::new(), HashMap::new());
    let mut n = 0;
    for bytes in files.iter().take(12) {
        let file = File::new(program, bytes);
        for at in (16..file.count()).step_by(16) {
            let range = at..(at + 16).min(file.count());
            let s = crate::solve::solve(&p, &comps, &file, range, 1, 256);
            let Some(leaf) = s.leaves.iter().find(|l| !l.clean) else { continue };
            n += 1;
            for (i, f) in leaf.starts[0].iter().enumerate() {
                if f.is_some() {
                    *split.entry(comps[i].op).or_default() += 1;
                }
            }
            for c in comps.iter().filter(|c| !c.in_place) {
                let w = leaf.end.get(c.at);
                let open = match program.ops[c.op] {
                    rerail_compiler::block::BlockOp::Prev(_, k) => w.k & ((1u64 << k) - 1) != (1u64 << k) - 1,
                    rerail_compiler::block::BlockOp::Toggle(_) => !w.full(),
                    _ => w.k & 1 == 0,
                };
                if open {
                    *ends.entry(c.op).or_default() += 1;
                }
            }
            let (values, _) = p.run_t(&file, at..(at + 16).min(file.count()), &{
                let mut b = p.unknown();
                let mut z = p.zero();
                crate::starts::in_place(&p, &comps, &file, at, &mut z);
                for c in comps.iter().filter(|c| c.in_place) {
                    b.set(c.at, crate::solve::T::known(c.at.get(&z)));
                }
                b
            });
            for &t in &program.totals {
                if values[usize::from(t)].iter().any(|w| !w.full()) {
                    *refusals.entry(usize::from(t)).or_default() += 1;
                }
            }
        }
    }
    let show = |x: usize| format!("#{x} {}", program.ops[x].text());
    eprintln!("== {name}: {n} chunks left unclean after splitting (cap 256)");
    let mut sp: Vec<_> = split.into_iter().collect();
    sp.sort();
    eprintln!("   split: {}", sp.iter().map(|(x, k)| format!("#{x}:{k}")).collect::<Vec<_>>().join(" "));
    let mut e: Vec<_> = ends.into_iter().collect();
    e.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (x, k) in e.iter().take(40) {
        let ops: Vec<String> = program.ops[*x].operands().iter().map(|&o| show(usize::from(o))).collect();
        eprintln!("   end open {k:4}  {}   <- {}", show(*x), ops.join(" | "));
    }
    let mut r: Vec<_> = refusals.into_iter().collect();
    r.sort_by(|a, b| b.1.cmp(&a.1));
    for (x, k) in r.iter().take(20) {
        eprintln!("   refusal open {k:4}  {}", show(*x));
    }
}

#[test]
#[ignore = "a report"]
fn report_the_open() {
    report_open("rr", rr(), &rr_files(65_536, usize::MAX));
    report_open("kotlin", kotlin(), &kotlin_files(65_536, usize::MAX));
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
