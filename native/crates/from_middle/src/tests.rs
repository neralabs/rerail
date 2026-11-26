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

/// What a start holds, by grammar, and the states a corpus reaches at 1 KB edges: the size of
/// what a chunk would solve for if it enumerated every state seen. A report, not a check
/// (`--ignored`); `RERAIL_KOTLIN_CORPUS` and `RERAIL_JSON_CORPUS` name the corpora.
fn report_states(name: &str, program: &BlockProgram, files: &[Vec<u8>]) {
    use std::collections::HashMap;
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let kind = |x: usize| match program.ops[x] {
        rerail_compiler::block::BlockOp::Prev(..) => "prev",
        rerail_compiler::block::BlockOp::Toggle(_) => "toggle",
        rerail_compiler::block::BlockOp::Escaped(_) => "escaped",
        rerail_compiler::block::BlockOp::Fill(..) => "fill",
        rerail_compiler::block::BlockOp::BackFill(..) => "back fill",
        _ => "other",
    };
    let mut by: HashMap<(&str, &str, bool), usize> = HashMap::new();
    for c in &comps {
        let at = match c.at {
            crate::starts::Place::Op(_) => "op",
            crate::starts::Place::Group(..) => "group",
            crate::starts::Place::Back(_) => "back",
        };
        *by.entry((at, kind(c.op), c.in_place)).or_default() += 1;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort();
    eprintln!("== {name}: {} ops, {} groups, {} components ({} in place)", program.ops.len(), p.groups.len(), comps.len(), comps.iter().filter(|c| c.in_place).count());
    for ((at, k, ip), n) in by {
        eprintln!("   {at:5} {k:9} {:9} {n}", if ip { "in place" } else { "state" });
    }
    // The states at 1 KB edges.
    let state_comps: Vec<usize> = (0..comps.len()).filter(|&i| !comps[i].in_place).collect();
    let mut seen: HashMap<Vec<u64>, usize> = HashMap::new();
    let mut nonzero = vec![0usize; state_comps.len()];
    let (mut edges, mut bytes_total) = (0usize, 0usize);
    for bytes in files {
        let file = File::new(program, bytes);
        bytes_total += bytes.len();
        for (at, s) in serial_starts(&p, &file, bytes).iter().enumerate() {
            if at == 0 || at % 16 != 0 || at >= file.count() {
                continue;
            }
            edges += 1;
            let st = crate::starts::state(&comps, s);
            for (j, &v) in st.iter().enumerate() {
                nonzero[j] += usize::from(v != 0);
            }
            *seen.entry(st).or_default() += 1;
        }
    }
    eprintln!("   {} files, {} bytes, {edges} edges at 1 KB: {} distinct states", files.len(), bytes_total, seen.len());
    let mut top: Vec<_> = seen.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1));
    for (st, n) in top.iter().take(12) {
        let set: Vec<String> = st.iter().enumerate().filter(|(_, v)| **v != 0).map(|(j, v)| format!("{}#{}={v:#x}", kind(comps[state_comps[j]].op), comps[state_comps[j]].op)).collect();
        eprintln!("   {n:6}  {}", if set.is_empty() { "all zero".to_owned() } else { set.join(" ") });
    }
    let ever: Vec<String> = nonzero.iter().enumerate().filter(|(_, n)| **n > 0).map(|(j, n)| format!("{}#{}:{n}", kind(comps[state_comps[j]].op), comps[state_comps[j]].op)).collect();
    eprintln!("   components ever set at an edge ({} of {}): {}", ever.len(), state_comps.len(), ever.join(" "));
}

#[test]
#[ignore = "a report over the corpora"]
fn report_the_states() {
    report_states("rr", rr(), &rr_files(65_536, usize::MAX));
    report_states("kotlin", kotlin(), &kotlin_files(65_536, usize::MAX));
    report_states("json", json(), &json_files(65_536, usize::MAX));
}

/// The design's question, measured: at a sample of 1 KB edges, the chunk run from every state
/// the corpus reaches at such edges (the true start's in-place components kept), block by
/// block, to the chunk's end. Each start is refused (a total's bit), converges (its start at an
/// edge inside the chunk is the true run's: the same from there on), or neither. Per edge: the
/// starts left, the starts left that never converge, and the distinct ends of the starts left
/// (one: the chunk's end is known whatever its start). Files tier 1 refuses are left out.
fn report_survivors(name: &str, program: &BlockProgram, files: &[Vec<u8>], edges_wanted: usize) {
    use std::collections::{HashMap, HashSet};
    let p = Prepared::new(program).expect("a program the chunks run");
    let comps = crate::starts::components(&p);
    let state_comps: Vec<usize> = (0..comps.len()).filter(|&i| !comps[i].in_place).collect();
    let totals: Vec<usize> = program.totals.iter().map(|&t| usize::from(t)).collect();
    // The edges, and the states the corpus reaches at them.
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut reached: HashSet<Vec<u64>> = HashSet::new();
    let mut serial: HashMap<usize, Vec<Start>> = HashMap::new();
    for (f, bytes) in files.iter().enumerate() {
        if program.run(bytes).totals.iter().any(|&t| t != 0) {
            continue;
        }
        let file = File::new(program, bytes);
        let starts = serial_starts(&p, &file, bytes);
        for at in (16..file.count()).step_by(16) {
            edges.push((f, at));
            reached.insert(crate::starts::state(&comps, &starts[at]));
        }
        serial.insert(f, starts);
    }
    let reached: Vec<Vec<u64>> = reached.into_iter().collect();
    let step = edges.len().div_ceil(edges_wanted.max(1)).max(1);
    let sample: Vec<(usize, usize)> = edges.iter().copied().step_by(step).collect();
    let (mut left_n, mut open_n, mut ends_n) = (Vec::new(), Vec::new(), Vec::new());
    let (mut refused_at, mut converged_at) = (vec![0usize; 17], vec![0usize; 17]);
    let mut truth_refused = 0usize;
    for &(f, at) in &sample {
        let file = File::new(program, &files[f]);
        let starts = &serial[&f];
        let end = (at + 16).min(file.count());
        let (mut left, mut open) = (0usize, 0usize);
        let mut ends: HashSet<Vec<u64>> = HashSet::new();
        for st in &reached {
            let mut s = starts[at].clone();
            for (j, &c) in state_comps.iter().enumerate() {
                comps[c].at.set(&mut s, st[j]);
            }
            let truth = crate::starts::state(&comps, &starts[at]) == *st;
            let mut outcome = None;
            for i in at..end {
                s.backs = starts[i].backs.clone();
                let (values, next) = p.run(&file, i..i + 1, &s);
                if totals.iter().any(|&t| values[t][0] != 0) {
                    outcome = Some(Err(i - at));
                    break;
                }
                s = next;
                if crate::starts::state(&comps, &s) == crate::starts::state(&comps, &starts[i + 1]) {
                    outcome = Some(Ok(i + 1 - at));
                    break;
                }
            }
            match outcome {
                Some(Err(j)) => {
                    refused_at[j.min(16)] += 1;
                    truth_refused += usize::from(truth);
                }
                Some(Ok(j)) => {
                    converged_at[j.min(16)] += 1;
                    left += 1;
                    ends.insert(crate::starts::state(&comps, &starts[end]));
                }
                None => {
                    left += 1;
                    open += 1;
                    ends.insert(crate::starts::state(&comps, &s));
                }
            }
        }
        left_n.push(left);
        open_n.push(open);
        ends_n.push(ends.len());
    }
    let pct = |v: &mut Vec<usize>, q: f64| -> usize {
        v.sort_unstable();
        v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0)
    };
    let ones = |v: &[usize]| v.iter().filter(|&&n| n == 1).count();
    eprintln!("== {name}: {} reached states; {} of {} edges sampled; the true start refused {truth_refused} times", reached.len(), sample.len(), edges.len());
    eprintln!("   starts left:      median {} p90 {} p99 {} max {}", pct(&mut left_n, 0.5), pct(&mut left_n, 0.9), pct(&mut left_n, 0.99), pct(&mut left_n, 1.0));
    eprintln!("   left, never converging: median {} p90 {} p99 {} max {}", pct(&mut open_n, 0.5), pct(&mut open_n, 0.9), pct(&mut open_n, 0.99), pct(&mut open_n, 1.0));
    eprintln!("   distinct ends:    median {} p90 {} p99 {} max {}; one end at {} of {} edges", pct(&mut ends_n, 0.5), pct(&mut ends_n, 0.9), pct(&mut ends_n, 0.99), pct(&mut ends_n, 1.0), ones(&ends_n), ends_n.len());
    eprintln!("   refused by block: {refused_at:?}");
    eprintln!("   converged by block: {converged_at:?}");
}

#[test]
#[ignore = "a report over the corpora"]
fn report_the_survivors() {
    report_survivors("rr", rr(), &rr_files(65_536, usize::MAX), 300);
    report_survivors("kotlin", kotlin(), &kotlin_files(65_536, usize::MAX), 300);
    report_survivors("json", json(), &json_files(1 << 20, usize::MAX), 300);
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
