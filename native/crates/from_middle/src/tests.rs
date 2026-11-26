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
