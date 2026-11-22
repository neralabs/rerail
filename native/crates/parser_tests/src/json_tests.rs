//! JSON's two reference forms (`json_parser/json-rfc.bitlex`, `jsonc.bitlex`) against
//! simdjson and tree-sitter-json (docs/parser-generators/21-json-against-simdjson.md).
//!
//! `ladder_tests` checks them as it checks every grammar. These add what only JSON has:
//! outside oracles, on files from outside.
//!
//! - **JSONTestSuite** (`RERAIL_JSON_SUITE`, its `test_parsing` directory): every `y_` file
//!   (valid JSON) is accepted by both forms; every `n_` file (invalid) is refused by `json-rfc`;
//!   `jsonc` refuses each too, unless it is valid once comments are allowed. Each file's
//!   compiled nodes equal the reference's.
//! - **A corpus** (`RERAIL_JSON_CORPUS`, simdjson's benchmark files): both forms accept each,
//!   with the same nodes, equal to tree-sitter-json's where it finds no error.
//! - **simdjson** (`RERAIL_SIMDJSON`, `tools/json-compare/simdjson_check`): its verdict on every
//!   file above, against `json-rfc`'s.
//!
//! And `json_bench`, CPU time per file of each tier and of tree-sitter-json.
//!
//! The files are not in the repository; each test runs by name with its variables set.

use crate::json::{base, files_in, form};
use rerail_cpu_stream::ladder::{Compiled, IdNode, as_ids, compile_step, reference};
use rerail_compiler::*;
use rerail_cpu_block::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};



/// JSON in simdjson's structure (`json_parser/json-simdjson.bitlex`, page 40), compiled.
fn simdjson_form() -> Compiled {
    let source = std::fs::read_to_string(std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../json_parser/json-simdjson.bitlex")).expect("reads");
    compile_step("json-simdjson", &source)
}

/// Page 40, stage 1: the simdjson form's tier 1 records a token where simdjson's stage 1 writes a
/// structural index, and nowhere else: its starts are simdjson's indexes, by count and by hash
/// (`fixtures/simdjson-stage1/expected.txt`, from `stage1_indexes.cpp`), on each file there.
#[test]
fn simdjson_forms_starts_are_simdjsons_indexes() {
    let c = simdjson_form();
    let block = &c.program.block_programs[0];
    let r = block.records.as_ref().expect("records");
    assert!(r.skip.is_none(), "tier 1 is the block of masks, not a token loop");
    // Tier 2 makes the records (`Records::fetched_in_tier2`): the CPU's tier 1 has no record
    // pass, and writes the starts alone, as simdjson's `bit_indexer::write`.
    assert!(r.fetched_in_tier2(), "tier 2 makes the records");
    assert!(c.parser.tier1.records.is_none(), "the CPU's tier 1 has no record pass");
    // Tier 2's tape is sized from the input once, as simdjson's: no room check a token.
    assert!(!c.parser.tier2.room_checked(), "no room check a token");
    // The scan for a string's next part only in the string loops, as simdjson's `parse_string`
    // only where a string is read: the `TOKEN`s whose switch takes the escapes.
    let native = rerail_compiler::native_ir::lower(&c.program, 0, 0).expect("lowers");
    let escapes: Vec<u16> = (0..9).collect();
    let loops: Vec<usize> = (0..native.structural.len())
        .filter(|&pc| matches!(native.structural[pc].operation, rerail_compiler::native_ir::StructuralOperation::Token { .. }))
        .filter(|&pc| match native.structural.get(pc + 1).map(|w| w.operation) {
            Some(rerail_compiler::native_ir::StructuralOperation::Switch { table }) => escapes.iter().all(|&e| native.switches[usize::from(table)].word(e).is_some()),
            _ => false,
        })
        .collect();
    assert!(!loops.is_empty(), "the string loops");
    assert_eq!(c.parser.tier2.part_scans, loops, "the part scan in the string loops alone");
    // Each part branches once, as `parse_string`'s: the scan's outcome goes straight to the place
    // the switch after would take it to, not back through the token's path and the switch.
    assert_eq!(c.parser.tier2.part_onward, loops, "every string loop's parts go straight on");
    // A number is read once, as `parse_number` reads it: its check (over the token's bytes
    // alone) made in the switch's edge that finds its end, from the one read of its bytes.
    let number_checks: Vec<usize> = (0..native.structural.len())
        .filter(|&pc| match native.structural[pc].operation {
            rerail_compiler::native_ir::StructuralOperation::Check { check } => {
                let lc = &native.local_checks[usize::from(check)];
                lc.token && lc.back == 0
            }
            _ => false,
        })
        .collect();
    assert!(!number_checks.is_empty(), "the numbers' checks");
    // At a check after a number's run: made there. At one after an atom (a keyword of the row):
    // decided by the atom's bytes as tier 2 compiles. Every check over a token's bytes is one.
    let mut either: Vec<usize> = c.parser.tier2.fused_checks.iter().chain(&c.parser.tier2.static_checks).copied().collect();
    either.sort_unstable();
    either.dedup();
    assert_eq!(either, number_checks, "every check of a number's row made where its token is found");
    assert!(!c.parser.tier2.fused_checks.is_empty() && !c.parser.tier2.static_checks.is_empty(), "both: {:?}, {:?}", c.parser.tier2.fused_checks, c.parser.tier2.static_checks);
    // The loop's state in registers, as simdjson's `src` and `next_structural`: no word of the
    // string loops or the continue states (the `TOKEN`s whose switch takes the `,` and no
    // escape) reads or writes the frame. Their cold calls keep the caller's registers
    // (`preserve_most`), and `ctx`'s fields are read from the pinned register in place.
    #[cfg(target_arch = "aarch64")]
    {
        use rerail_compiler::native_ir::StructuralOperation;
        let continues = (0..native.structural.len()).filter(|&pc| matches!(native.structural[pc].operation, StructuralOperation::Token { .. })).filter(|&pc| match native.structural.get(pc + 1).map(|w| w.operation) {
            Some(StructuralOperation::Switch { table }) => native.switches[usize::from(table)].word(14).is_some() && native.switches[usize::from(table)].word(0).is_none(),
            _ => false,
        });
        let hot: Vec<u32> = loops.iter().copied().chain(continues).flat_map(|pc| [pc as u32, pc as u32 + 1]).collect();
        assert!(hot.len() > loops.len() * 2, "the continue states");
        let t2 = &c.parser.tier2;
        for &(pc, from, to) in t2.word_ranges.iter().filter(|r| hot.contains(&r.0)) {
            for at in (from as usize..to as usize).step_by(4) {
                let w = u32::from_le_bytes(t2.code_bytes[at..at + 4].try_into().unwrap());
                // A load or store (op0 `x1x0`), not a literal load (no base), with `sp` its base.
                let on_frame = (w >> 25) & 0b101 == 0b100 && (w >> 24) & 0x3b != 0x18 && (w >> 5) & 31 == 31;
                assert!(!on_frame, "word {pc} reads or writes the frame at {at:#x}: {w:#010x}");
            }
        }
    }
    // Nor a row of what tier 2 reads from the bytes (the run's mask, the string's quote), or
    // not at all: the string's escape part, as tier 2 reads a string as simdjson's
    // `parse_string` does (first quote or backslash), with no escape carry.
    let unstored = rerail_compiler::block_native::BlockNative::lower(block).cpu_unstored();
    let read: Vec<usize> = r
        .rules
        .iter()
        .filter_map(|x| match *x {
            rerail_compiler::records::EndRule::Run(o) => Some(usize::from(o)),
            _ => None,
        })
        .chain(r.readers.iter().flat_map(|rd| rd.arms.iter().map(|a| usize::from(a.closer)).chain(rd.parts.iter().map(|p| usize::from(p.start)))))
        .collect();
    assert!(!read.is_empty() && read.iter().all(|o| unstored.contains(o)), "the CPU's tier 1 stores no row tier 2 makes from the bytes: {read:?} against {unstored:?}");
    for line in crate::block_tests::STAGE1_EXPECTED.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (path, len, n, hash) = (f[0], f[1].parse::<usize>().unwrap(), f[2].parse::<usize>().unwrap(), u64::from_str_radix(f[3], 16).unwrap());
        if len > 200_000 {
            continue;
        }
        let bytes = crate::block_tests::repo_file(path);
        let run = block.run(&bytes);
        // A file stage 1's edges hold need not be valid JSON: its starts are compared all the same.
        eprintln!("{path}: tier 1 {}", if run.totals.iter().any(|&t| t != 0) { "refuses it" } else { "accepts it" });
        let starts = &run.positions[usize::from(r.starts)];
        assert_eq!((starts.len(), crate::block_tests::fnv(starts)), (n, hash), "{path}: tier 1's starts against simdjson's indexes");
        let cpu = c.parser.tier1.run_files(&[&bytes]).remove(0);
        let cpu_starts = &cpu.positions[usize::from(r.starts)];
        assert_eq!((cpu_starts.len(), crate::block_tests::fnv(cpu_starts)), (n, hash), "{path}: the CPU's tier 1's starts against simdjson's indexes");
        assert!(cpu.positions[usize::from(r.lasts)].is_empty() && cpu.codes.is_empty(), "{path}: the CPU's tier 1 writes no lasts and no codes");
    }
}

/// Page 40, stage 2: the simdjson form's trees are `json-rfc`'s, by the interpreter and by
/// Cranelift, though its tier 1 records only simdjson's structurals and tier 2 reads each
/// string's parts. Strings with escapes at every offset of a block's edge, invalid documents,
/// and the corpus (`RERAIL_JSON_CORPUS`, where set).
#[test]
fn simdjson_forms_trees_are_14_jsons() {
    let (sj, j14) = (simdjson_form(), form("json-rfc"));
    let inputs = tree_inputs();
    let (mut parsed, mut refused) = (0, 0);
    for (k, f) in inputs.iter().enumerate() {
        let show = || String::from_utf8_lossy(&f[..f.len().min(80)]).into_owned();
        let want = ours(&j14, f);
        let by_interp = stream::run(&sj.program, f, &sj.program.block_programs[0].run(f)).ok().map(|n| n.iter().map(|x| (x.kind, x.start, x.end)).collect::<Vec<_>>());
        let want_interp = stream::run(&j14.program, f, &j14.program.block_programs[0].run(f)).ok().map(|n| n.iter().map(|x| (x.kind, x.start, x.end)).collect::<Vec<_>>());
        assert_eq!(by_interp.is_some(), want_interp.is_some(), "file {k} ({:?}): the interpreter's verdict", show());
        assert_eq!(ours(&sj, f), want, "file {k} ({:?}): Cranelift's nodes", show());
        if want.is_some() { parsed += 1 } else { refused += 1 }
    }
    eprintln!("{} files: {parsed} parsed, {refused} refused, by both forms alike", inputs.len());
}

/// The JIT's format 16 run of the simdjson form passes each node to the writer's ops at its word (`direct`,
/// `native_cpu_facts::direct_keys`): no working row, so no step on the rows is called, and its tape holds no format 15
/// buffer. (Kept from the Rust trees' test, which 0555 removed.)
#[test]
fn simdjson_forms_sixteen_run_is_direct() {
    let jit = simdjson_form();
    let s16 = jit.parser.sixteen().expect("a format 16 tier 2");
    assert!(s16.direct && s16.row_calls == 0, "the JIT's format 16 run: direct {}, {} calls of a step on the rows", s16.direct, s16.row_calls);
    assert_eq!(s16.tape(1000).held(), [0, 0], "the direct run's tape: no format 15 buffer, no working row");
}

/// Values nested `n` deep for each `n`: arrays, objects, and an object's value an array's by turns.
fn deep_nests(depths: std::ops::Range<usize>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for n in depths {
        out.push(format!("{}1{}", "[".repeat(n), "]".repeat(n)).into_bytes());
        out.push(format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n)).into_bytes());
        let (open, close): (String, String) = (0..n).map(|k| if k % 2 == 0 { ("[", "]") } else { ("{\"k\": ", "}") }).unzip();
        out.push(format!("{open}1{}", close.chars().rev().collect::<String>()).into_bytes());
    }
    out
}

/// The trees test's files: the made-up cases, then the corpus (`RERAIL_JSON_CORPUS`) where set.
fn tree_inputs() -> Vec<Vec<u8>> {
    let mut inputs: Vec<Vec<u8>> = vec![
        br#"{"a": [1, -2.5e3, true, null], "b\n": {"c": "dAx"}}"#.to_vec(),
        br#"["", "\\", "a\"b", "\\\"", "x", "\n\t", "true", "\u00e9\ud83d\ude00"]"#.to_vec(),
        br#""open"#.to_vec(),
        br#"["\q"]"#.to_vec(),
        br#"[1 2]"#.to_vec(),
        br#"{"a" 1}"#.to_vec(),
        br#"[tru]"#.to_vec(),
        br#"[01]"#.to_vec(),
        br#"  "s"  "#.to_vec(),
        b"\xef\xbb\xbf[1]".to_vec(),
        // Newlines after a stripped prefix: the tape's newline index from tier 1's bits, offset by it.
        b"\xef\xbb\xbf[1,\n2,\n\n\"a\"]\n".to_vec(),
        b"\n\n{\"k\":\n[true,\nnull]}\n\n".to_vec(),
        // Atoms as simdjson tests them (page 40, step 3): at the file's end, a run longer than
        // the word, a word cut short, before a structural and before whitespace.
        br#"true"#.to_vec(),
        br#"null"#.to_vec(),
        br#"[truex]"#.to_vec(),
        br#"[nul]"#.to_vec(),
        br#"{"k":false,"t":true}"#.to_vec(),
        br#"[null ,true	]"#.to_vec(),
        br#"["\u0041true", "null\n", "fals"]"#.to_vec(),
        // A continue state takes only its punctuation (`,` and `]`, or `,` and `}`), each
        // straight to its target; every other token there is refused at once.
        br#"[1 true]"#.to_vec(),
        br#"[1 "x"]"#.to_vec(),
        br#"[1 {}]"#.to_vec(),
        br#"[1,]"#.to_vec(),
        br#"[1:2]"#.to_vec(),
        br#"{"a":1 true}"#.to_vec(),
        br#"{"a":1,}"#.to_vec(),
        br#"{"a":1]"#.to_vec(),
        br#"{"a",1}"#.to_vec(),
        br#"{"a":[1}"#.to_vec(),
        br#"[[],{},[1,{"b":[]}]]"#.to_vec(),
        br#"[] x"#.to_vec(),
        // A number checked in the pass that finds its end (step 12): each value place, good and
        // bad, at the file's end, and longer than the 16 bytes read at once.
        br#"[1.]"#.to_vec(),
        br#"[-]"#.to_vec(),
        br#"[1e]"#.to_vec(),
        br#"[1.5e+]"#.to_vec(),
        br#"[1x,2]"#.to_vec(),
        br#"[-0.25e-7,0,-0,1E+2]"#.to_vec(),
        br#"{"a":01}"#.to_vec(),
        br#"{"a":-1.5E3,"b":2}"#.to_vec(),
        br#"-12"#.to_vec(),
        br#"1."#.to_vec(),
        br#"[123456789012345]"#.to_vec(),
        br#"[1234567890123456,12345678901234567]"#.to_vec(),
        br#"[1.234567890123456e+10]"#.to_vec(),
        br#"[1.23456789012345.6]"#.to_vec(),
        // `\u` escapes that fail its checks, beside the other escapes, which skip them.
        br#"["\u12g4"]"#.to_vec(),
        br#"["\ud800"]"#.to_vec(),
        br#"["\ud800\u0041"]"#.to_vec(),
        br#"["\n\ud83d\ude00\t", "\u004"]"#.to_vec(),
        br#"["\n\ud83d\ude00\t\\", "\/\u00e9\"\b"]"#.to_vec(),
    ];
    for pad in [0usize, 1, 61, 62, 63, 64, 65, 127] {
        for n in [0usize, 1, 62, 63, 64, 130] {
            inputs.push(format!("{}[\"{}\\n{}\", \"\\\\\\\"{}\", {}]", " ".repeat(pad), "s".repeat(n), "t".repeat(n), "u".repeat(n), "9".repeat(n.max(1))).into_bytes());
        }
    }
    // Past a block (1,024 nodes) and past a cell's width (the tests' widths are a byte): the writer's block switch,
    // a block made final by the next block's first node and by a late close, an ancestor closed blocks after it
    // opened, and values to the side lists (a gap, a length and an inner node's size past 255).
    inputs.push(format!("[{}]", vec!["true"; 3000].join(",")).into_bytes());
    inputs.push(format!("{}1{}", "[".repeat(400), "]".repeat(400)).into_bytes());
    // At the depth's limit (`DEPTH`, 1,024 calls) and past it, arrays and objects and both by turns: the open nodes
    // the writer's stack is reserved for (`native_cpu_facts::open_bound`, decision 0537), at their most; past it, refused.
    inputs.extend(deep_nests(1022..1028));
    inputs.push(format!("{{{}}}", (0..1500).map(|k| format!("\"k{k}\": [{k}, \"v\\n\"]")).collect::<Vec<_>>().join(",")).into_bytes());
    inputs.push(format!("[{}1,{}\"{}\", [{}]]", " ".repeat(300), "\n".repeat(400), "s".repeat(700), vec!["null"; 300].join(", ")).into_bytes());
    inputs.push(format!("[{}]", (0..1100).map(|k| if k % 97 == 0 { format!("{}{k}", " ".repeat(260)) } else { k.to_string() }).collect::<Vec<_>>().join(",")).into_bytes());
    if std::env::var("RERAIL_JSON_CORPUS").is_ok() {
        inputs.extend(files_in("RERAIL_JSON_CORPUS").iter().map(|p| std::fs::read(p).expect("reads")));
    }
    inputs
}

/// Page 40, order item 4: the simdjson form end to end against simdjson, on simdjson's own inputs (`RERAIL_JSON_FILES`,
/// `:`-separated; `twitter.json` and `numbers.json`). Ours: the Rust the Bitlex compiler writes from the grammar
/// (`rerail-json-aot`, decision 0551), tier 1 and tier 2, writing format 16 with the tape's home and presence, as
/// built here (`--profile profiling`: optimized). simdjson's: its DOM parser, stage 1 and stage 2 to its tape, from a
/// padded copy of the bytes (`structurals`). Per file: the instructions retired a byte and the time a byte (each the
/// median of `RERAIL_ITERS` runs), single-threaded.
#[cfg(target_arch = "aarch64")]
#[test]
#[ignore = "a measurement: run by name with RERAIL_JSON_FILES and --nocapture, in an optimized build"]
fn simdjson_form_against_simdjson() {
    let files: Vec<(String, Vec<u8>)> = std::env::var("RERAIL_JSON_FILES")
        .expect("RERAIL_JSON_FILES")
        .split(':')
        .map(|p| (base(Path::new(p)), std::fs::read(p).expect("reads")))
        .collect();
    let iters: usize = std::env::var("RERAIL_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let jit = simdjson_form();
    let (_, tag) = jit.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), rerail_json_aot::WIDTHS, 1).expect("a language");
    let numbers = jit.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
    // The process's instructions retired (`native_cpu_codegen::retired`: macOS's count, zero elsewhere).
    let retired = || rerail_compiler::native_cpu_codegen::retired().0 as f64;
    // The median of each run's instructions and time.
    let measure = |run: &mut dyn FnMut()| {
        run();
        let (mut ins, mut ns) = (Vec::new(), Vec::new());
        for _ in 0..iters {
            let (i, t) = (retired(), thread_ns());
            run();
            ns.push(thread_ns() - t);
            ins.push(retired() - i);
        }
        (median(ins), median(ns))
    };
    println!("simdjson's implementation: {}", rerail_simdjson_native::implementation());
    println!("file\tbytes\tours_ins_per_byte\tsimdjson_ins_per_byte\tours_GB_s\tsimdjson_GB_s\tins_ratio\ttime_ratio");
    for (name, f) in &files {
        let nodes = f.len() / 2 + 16;
        // Ours writes the JIT's tape.
        let want = jit.parser.image16(f, &lang, &numbers, false).map(|w| w.extent().to_vec()).unwrap_or_else(|e| panic!("{name}: the JIT refuses it: {e:?}"));
        // The file padded once, as simdjson's caller pads it (`padded_string`).
        let fp = rerail_json_aot::padded(f);
        let mut w = rerail_tape_format::f16::Writer::new(&lang, f, tag, false, true, nodes);
        rerail_json_aot::parse(&fp, f.len(), &mut w).unwrap_or_else(|b| panic!("{name}: the Rust refuses it at byte {b}"));
        assert!(w.finish().expect("finishes").tape.extent() == &want[..], "{name}: the Rust's tape is the JIT's");
        // Ours as simdjson's caller runs simdjson: one writer, kept from one document to the next (simdjson's parser is
        // kept by `structurals`), each tape given back as the next one's room.
        let mut w = rerail_tape_format::f16::Writer::empty();
        let mut ours = || {
            w.begin(&lang, f, tag, false, true, nodes);
            rerail_json_aot::parse(&fp, f.len(), &mut w).expect("parses");
            let t = w.finish().expect("finishes").tape;
            w.recycle(std::hint::black_box(t));
        };
        // `RERAIL_SPIN`: ours alone, for that many seconds, for a profiler (the process's id printed first).
        if let Some(secs) = std::env::var("RERAIL_SPIN").ok().and_then(|v| v.parse::<u64>().ok()) {
            eprintln!("pid {}", std::process::id());
            let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            while std::time::Instant::now() < until {
                ours();
            }
            return;
        }
        let (oi, on) = measure(&mut ours);
        let (si, sn) = measure(&mut || {
            std::hint::black_box(rerail_simdjson_native::structurals(f, |i| i.len()).expect("simdjson parses it"));
        });
        // Of ours, tier 1 alone; and tier 2 alone, on tier 1's records made once, the writer kept.
        let (t1i, t1n) = measure(&mut || {
            std::hint::black_box(rerail_json_aot::lex(&fp, f.len()));
        });
        let lexed = rerail_json_aot::lex(&fp, f.len());
        let (t2i, t2n) = measure(&mut || {
            w.begin(&lang, f, tag, false, true, nodes);
            rerail_json_aot::parse_lexed(&fp, &lexed, &mut w).expect("parses");
            let t = w.finish().expect("finishes").tape;
            w.recycle(std::hint::black_box(t));
        });
        let b = f.len() as f64;
        println!("{name}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}", f.len(), oi / b, si / b, b / on, b / sn, oi / si, on / sn);
        println!("  of ours, ins a byte: tier 1 {:.2} ({:.3} ns), tier 2 and the writer {:.2} ({:.3} ns)", t1i / b, t1n / b, t2i / b, t2n / b);
    }
}

fn ours(c: &Compiled, bytes: &[u8]) -> Option<Vec<IdNode>> {
    c.parser.parse(bytes).ok().map(as_ids)
}

/// simdjson's verdict on each file (`RERAIL_SIMDJSON`, the checker), or none without it.
fn simdjson(paths: &[PathBuf]) -> Option<HashMap<PathBuf, bool>> {
    let tool = std::env::var("RERAIL_SIMDJSON").ok()?;
    let mut out = HashMap::new();
    for chunk in paths.chunks(200) {
        let run = std::process::Command::new(&tool).arg("verdict").args(chunk).output().expect("runs simdjson_check");
        let text = String::from_utf8(run.stdout).expect("text");
        for (line, path) in text.lines().zip(chunk) {
            assert!(line[2..].starts_with(&*path.to_string_lossy()), "{line}");
            out.insert(path.clone(), line.starts_with('1'));
        }
    }
    Some(out)
}

/// A node by names: kind, field, start, end, size.
type Named = (String, String, usize, usize, usize);

/// tree-sitter-json's nodes of `bytes`, or `None` where it finds an error.
fn tree_sitter_json(bytes: &[u8]) -> Option<Vec<Named>> {
    tree_sitter_json_with(bytes, false)
}

/// tree-sitter-json's nodes; with `tokens`, its anonymous tokens too.
fn tree_sitter_json_with(bytes: &[u8], tokens: bool) -> Option<Vec<Named>> {
    let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).expect("tree-sitter-json");
    let tree = parser.parse(bytes, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    // Named-only, as the engine's tapes: an anonymous token (`{`, `,`, `"` …) is left out; a
    // comment, an extra, stays.
    fn walk(n: tree_sitter::Node<'_>, field: Option<&str>, tokens: bool, out: &mut Vec<Named>) -> usize {
        if !tokens && !n.is_named() && !n.is_extra() {
            return 0;
        }
        let at = out.len();
        out.push((n.kind().to_owned(), field.unwrap_or("").to_owned(), n.start_byte(), n.end_byte(), 0));
        let mut size = 1;
        let mut c = n.walk();
        if c.goto_first_child() {
            loop {
                size += walk(c.node(), c.field_name(), tokens, out);
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        out[at].4 = size;
        size
    }
    let mut out = Vec::new();
    walk(tree.root_node(), None, tokens, &mut out);
    Some(out)
}

/// Ids back to names, for comparing with tree-sitter.
fn named(c: &Compiled, nodes: &[IdNode]) -> Vec<Named> {
    let (kinds, fields) = c.ids.names();
    nodes
        .iter()
        .map(|n| {
            let field = if n.1 == 0 { String::new() } else { fields[&n.1].clone() };
            (kinds[&n.0].clone(), field, n.3, n.4, n.5)
        })
        .collect()
}


/// JSONTestSuite: the forms' verdicts by the suite's (`y_` valid, `n_` invalid, `i_` either),
/// with simdjson's and tree-sitter-json's beside them.
#[test]
#[ignore = "needs JSONTestSuite: run by name with RERAIL_JSON_SUITE (and RERAIL_SIMDJSON)"]
fn json_suite_accuracy() {
    // The reference's tree parser recurses per nesting level; the suite nests 100,000 deep.
    std::thread::Builder::new().stack_size(1 << 30).spawn(suite).expect("a thread").join().expect("the suite");
}

fn suite() {
    let (json, jsonc) = (form("json-rfc"), form("jsonc"));
    let paths = files_in("RERAIL_JSON_SUITE");
    let simd = simdjson(&paths);
    let mut problems: Vec<String> = Vec::new();
    // By the suite's class: [files, `json-rfc` accepts, `jsonc` accepts, simdjson accepts, tree-sitter accepts].
    let mut table: std::collections::BTreeMap<char, [usize; 5]> = std::collections::BTreeMap::new();
    let mut differ_simd: Vec<String> = Vec::new();
    let mut jsonc_takes: Vec<String> = Vec::new();
    let mut ts_wrong: Vec<String> = Vec::new();
    for path in &paths {
        let bytes = std::fs::read(path).expect("reads");
        let name = base(path);
        let class = name.chars().next().expect("a class");
        let (a, b) = (ours(&json, &bytes), ours(&jsonc, &bytes));
        // The compiled path equals the reference on every file.
        for (c, got, which) in [(&json, &a, "14"), (&jsonc, &b, "15")] {
            if reference(c, &bytes) != *got {
                problems.push(format!("{name}: step {which} compiled differs from the reference"));
            }
        }
        let s = simd.as_ref().map(|m| m[path]);
        let t = tree_sitter_json(&bytes).is_some();
        let row = table.entry(class).or_default();
        row[0] += 1;
        row[1] += usize::from(a.is_some());
        row[2] += usize::from(b.is_some());
        row[3] += usize::from(s == Some(true));
        row[4] += usize::from(t);
        match class {
            'y' if a.is_none() || b.is_none() => problems.push(format!("{name}: valid JSON refused (14: {}, 15: {})", a.is_some(), b.is_some())),
            'n' if a.is_some() => problems.push(format!("{name}: invalid JSON accepted by `json-rfc`")),
            'n' if b.is_some() => jsonc_takes.push(name.clone()),
            _ => {}
        }
        if a.is_some() && b.is_some() && a != b {
            problems.push(format!("{name}: the two forms' nodes differ"));
        }
        if (class == 'y' && !t) || (class == 'n' && t) {
            ts_wrong.push(name.clone());
        }
        if let Some(s) = s
            && s != a.is_some()
        {
            differ_simd.push(format!("{name} (simdjson {s}, `json-rfc` {})", a.is_some()));
        }
    }
    eprintln!("class  files  step14  step15  simdjson  tree-sitter-json   (files accepted)");
    for (c, r) in &table {
        eprintln!("{c}_     {:5}  {:6}  {:6}  {:8}  {:5}", r[0], r[1], r[2], r[3], r[4]);
    }
    eprintln!("`jsonc` accepts, of the n_ files: {jsonc_takes:?}");
    eprintln!("tree-sitter-json refuses valid or accepts invalid: {ts_wrong:?}");
    eprintln!("`json-rfc` and simdjson differ on {}: {differ_simd:#?}", differ_simd.len());
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// simdjson's benchmark files: both forms accept each, with the same nodes, and the nodes are
/// tree-sitter-json's where it finds no error.
#[test]
#[ignore = "needs a corpus: run by name with RERAIL_JSON_CORPUS (and RERAIL_SIMDJSON)"]
fn json_corpus_accuracy() {
    let (json, jsonc) = (form("json-rfc"), form("jsonc"));
    let paths = files_in("RERAIL_JSON_CORPUS");
    let simd = simdjson(&paths);
    let mut problems: Vec<String> = Vec::new();
    for path in &paths {
        let bytes = std::fs::read(path).expect("reads");
        let name = base(path);
        let (a, b) = (ours(&json, &bytes), ours(&jsonc, &bytes));
        let ts = tree_sitter_json(&bytes);
        let s = simd.as_ref().map(|m| m[path]);
        let same_ts = match (&a, &ts) {
            (Some(n), Some(t)) => Some(named(&json, n) == *t),
            _ => None,
        };
        eprintln!(
            "{name}: {} bytes; `json-rfc` {}, `jsonc` {}, simdjson {s:?}, tree-sitter {}; nodes {}; equal to tree-sitter's: {same_ts:?}",
            bytes.len(),
            a.is_some(),
            b.is_some(),
            ts.is_some(),
            a.as_ref().map_or(0, Vec::len)
        );
        if a.is_none() || b.is_none() {
            problems.push(format!("{name}: refused"));
        }
        if a != b {
            problems.push(format!("{name}: the forms' nodes differ"));
        }
        if same_ts == Some(false) {
            let (n, t) = (named(&json, a.as_ref().expect("accepted")), ts.expect("parsed"));
            let k = n.iter().zip(&t).position(|(x, y)| x != y).unwrap_or(n.len().min(t.len()));
            problems.push(format!("{name}: node {k} differs from tree-sitter's: {:?} against {:?}", n.get(k), t.get(k)));
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

fn thread_ns() -> f64 {
    native_cpu_block::thread_cpu() * 1e9
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() / 2]
}

/// CPU time per file (the median of `RERAIL_ITERS` runs): each form's tier 1, tier 2, and
/// tree-sitter-json's parse. Buffers are laid out once; only the calls are timed.
///
/// With `RERAIL_BENCH_RUN` set (`none`, `tier1`, `both`, `tree-sitter`) it instead runs that
/// many times over every file and prints nothing else, for counting instructions with
/// `/usr/bin/time -l` (the difference from `none`).
#[test]
#[ignore = "a measurement: run by name with RERAIL_JSON_CORPUS and --nocapture"]
fn json_bench() {
    // `RERAIL_OPT`: Cranelift's optimizer on tier 1 (`1`), tier 2 (`2`), or both (`1,2`).
    let opt = std::env::var("RERAIL_OPT").unwrap_or_default();
    // The compiles' options are the run's (decision 0442).
    let _run = rerail_context::test_run();
    let options = rerail_compiler::native_cpu_codegen::options();
    options.optimize_tier1.store(opt.split(',').any(|t| t == "1"), std::sync::atomic::Ordering::Relaxed);
    options.optimize_tier2.store(opt.split(',').any(|t| t == "2"), std::sync::atomic::Ordering::Relaxed);
    let iters: usize = std::env::var("RERAIL_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    // `RERAIL_FORMS`: other Bitlex files to time, as paths, in place of the two forms.
    let forms: Vec<(String, Compiled)> = match std::env::var("RERAIL_FORMS") {
        Ok(list) => list
            .split(',')
            .map(|p| {
                let name = Path::new(p).file_stem().expect("a name").to_string_lossy().into_owned();
                let c = compile_step(&name, &std::fs::read_to_string(p).expect("reads"));
                (name, c)
            })
            .collect(),
        Err(_) => vec![("json-rfc".to_owned(), form("json-rfc")), ("jsonc".to_owned(), form("jsonc"))],
    };
    let paths = files_in("RERAIL_JSON_CORPUS");
    let files: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).expect("reads")).collect();
    let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let mut ts = tree_sitter::Parser::new();
    ts.set_language(&grammar).expect("tree-sitter-json");
    if let Ok(run) = std::env::var("RERAIL_BENCH_RUN") {
        let c = &forms[usize::from(std::env::var("RERAIL_FORM").is_ok_and(|f| f == "15"))].1;
        let mut prepared: Vec<_> = files.iter().map(|f| native_cpu_block::Batch::new(&c.parser.tier1, &[f])).collect();
        let mut tapes: Vec<_> = prepared
            .iter_mut()
            .map(|b| {
                b.call(&c.parser.tier1);
                c.parser.tape(b)
            })
            .collect();
        let mut ok = 0usize;
        for _ in 0..iters {
            for ((b, t), f) in prepared.iter_mut().zip(&mut tapes).zip(&files) {
                match run.as_str() {
                    "tier1" => b.call(&c.parser.tier1),
                    "both" => {
                        b.call(&c.parser.tier1);
                        ok += usize::from(c.parser.words(b, t).is_ok());
                    }
                    "tree-sitter" => ok += usize::from(ts.parse(f, None).is_some()),
                    _ => {}
                }
            }
        }
        println!("{ok}");
        return;
    }
    println!("file\tbytes\tform\ttier1_ns\ttier2_ns\ttree_sitter_ns");
    for (path, f) in paths.iter().zip(&files) {
        let t_ts = median(
            (0..iters.min(5))
                .map(|_| {
                    let t = thread_ns();
                    assert!(ts.parse(f, None).is_some());
                    thread_ns() - t
                })
                .collect(),
        );
        for (name, c) in &forms {
            let mut batch = native_cpu_block::Batch::new(&c.parser.tier1, &[f]);
            batch.call(&c.parser.tier1);
            let mut tape = c.parser.tape(&batch);
            let (mut one, mut two) = (Vec::new(), Vec::new());
            for _ in 0..iters {
                let t = thread_ns();
                batch.call(&c.parser.tier1);
                one.push(thread_ns() - t);
                let t = thread_ns();
                let r = c.parser.words(&batch, &mut tape);
                two.push(thread_ns() - t);
                assert!(r.is_ok(), "{}: {name} refuses", base(path));
            }
            println!("{}\t{}\t{name}\t{:.0}\t{:.0}\t{t_ts:.0}", base(path), f.len(), median(one), median(two));
        }
    }
}

/// JSON's tier 2 as written, for reading it next to simdjson's stage 2: each word's operation,
/// a switch's targets by byte (runs of bytes to one target), and the code (CLIF, machine code,
/// each word's byte ranges in it, and the simdjson form's Rust). Into `RERAIL_DUMP_TIER2`; `RERAIL_FORM`.
#[test]
#[ignore = "a dump: run by name with RERAIL_DUMP_TIER2"]
fn json_tier2_by_word() {
    let name = std::env::var("RERAIL_FORM").unwrap_or_else(|_| "json-rfc".to_owned());
    let c = if name == "simdjson" { simdjson_form() } else { form(&name) };
    let native = rerail_compiler::native_ir::lower(&c.program, 0, 0).expect("lowers");
    let dir = std::env::var("RERAIL_DUMP_TIER2").expect("RERAIL_DUMP_TIER2");
    std::fs::write(format!("{dir}/{name}.clif"), &c.parser.tier2.clif).expect("writes");
    std::fs::write(format!("{dir}/{name}.s"), &c.parser.tier2.disassembly).expect("writes");
    std::fs::write(format!("{dir}/{name}.bin"), &c.parser.tier2.code_bytes).expect("writes");
    let ranges: String = c.parser.tier2.word_ranges.iter().map(|&(pc, a, b)| format!("{pc}\t{a}\t{b}\n")).collect();
    std::fs::write(format!("{dir}/{name}.ranges"), ranges).expect("writes");
    let show = |b: u8| if b.is_ascii_graphic() { format!("{}", b as char) } else { format!("{b:#04x}") };
    let mut words = String::new();
    for (pc, w) in native.structural.iter().enumerate() {
        words += &format!("{pc}\t{:?}\n", w.operation);
        if let rerail_compiler::native_ir::StructuralOperation::Switch { table } = w.operation {
            let s = &native.switches[usize::from(table)];
            let mut b = 0usize;
            while b < 256 {
                let t = s.targets[b];
                let mut e = b;
                while e + 1 < 256 && s.targets[e + 1] == t {
                    e += 1;
                }
                if t != rerail_compiler::stream::REFUSE {
                    words += &format!("\t\t{}..{} -> {t}\n", show(b as u8), show(e as u8));
                }
                b = e + 1;
            }
            words += &format!("\t\tend -> {}{}\n", s.end, if s.words.is_empty() { String::new() } else { format!(", words {:?}", s.words) });
        }
        // A check: a token's (over its bytes) or at a tier-1 mask's bits, and its ops.
        if let rerail_compiler::native_ir::StructuralOperation::Check { check } = w.operation {
            let c = &native.local_checks[usize::from(check)];
            let at = if c.token { "the token's bytes".to_owned() } else { format!("mask {}", c.mask) };
            words += &format!("\t\t{at}, delay {}, back {}: {:?}\n", c.delay, c.back, c.ops);
        }
    }
    std::fs::write(format!("{dir}/{name}.words"), words).expect("writes");
    // The simdjson form's tier 2 as the Bitlex compiler writes it from the grammar (`rerail-json-aot`, decision 0551),
    // to read beside simdjson's stage 2.
    if name == "simdjson" {
        std::fs::write(format!("{dir}/{name}.rs"), json_rust().1).expect("writes");
    }
}

/// Cranelift's optimizer on JSON (doc 28): `json-rfc` compiled four ways (optimizer off, on tier
/// 1, on tier 2, on both) in one process, then timed in turns: each round runs every form over
/// every file, tier 1 then tier 2, and each form keeps its best round. In turns, so the
/// machine's load falls on every form alike.
#[test]
#[ignore = "a benchmark: run by name with RERAIL_JSON_CORPUS"]
fn json_optimizer_bench() {
    use std::sync::atomic::Ordering::Relaxed;
    let rounds: usize = std::env::var("RERAIL_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let name = std::env::var("RERAIL_FORM").unwrap_or_else(|_| "json-rfc".to_owned());
    let paths = files_in("RERAIL_JSON_CORPUS");
    let files: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).expect("reads")).collect();
    let bytes: usize = files.iter().map(Vec::len).sum();
    // The compiles' options are the run's (decision 0442).
    let _run = rerail_context::test_run();
    let options = rerail_compiler::native_cpu_codegen::options();
    let forms: Vec<(&str, Compiled)> = [("off", false, false), ("tier 1", true, false), ("tier 2", false, true), ("both", true, true)]
        .into_iter()
        .map(|(label, one, two)| {
            options.optimize_tier1.store(one, Relaxed);
            options.optimize_tier2.store(two, Relaxed);
            (label, form(&name))
        })
        .collect();
    options.optimize_tier1.store(false, Relaxed);
    options.optimize_tier2.store(false, Relaxed);
    // Each form's batches and tapes, made once.
    let mut state: Vec<Vec<_>> = forms
        .iter()
        .map(|(_, c)| {
            files
                .iter()
                .map(|f| {
                    let mut b = native_cpu_block::Batch::new(&c.parser.tier1, &[f]);
                    b.call(&c.parser.tier1);
                    let t = c.parser.tape(&b);
                    (b, t)
                })
                .collect()
        })
        .collect();
    // Each tier's CPU time, instructions retired and cycles, each its least over the rounds.
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let mut best = vec![[(f64::MAX, u64::MAX, u64::MAX); 2]; forms.len()];
    for _ in 0..rounds {
        for (k, (_, c)) in forms.iter().enumerate() {
            let mut sum = [(0.0, 0u64, 0u64); 2];
            for (b, t) in &mut state[k] {
                let (i0, c0, s) = (retired().0, retired().1, thread_ns());
                b.call(&c.parser.tier1);
                let (s1, (i1, c1)) = (thread_ns(), retired());
                assert!(c.parser.words(b, t).is_ok());
                let (s2, (i2, c2)) = (thread_ns(), retired());
                sum[0] = (sum[0].0 + s1 - s, sum[0].1 + i1 - i0, sum[0].2 + c1 - c0);
                sum[1] = (sum[1].0 + s2 - s1, sum[1].1 + i2 - i1, sum[1].2 + c2 - c1);
            }
            for j in 0..2 {
                best[k][j] = (best[k][j].0.min(sum[j].0), best[k][j].1.min(sum[j].1), best[k][j].2.min(sum[j].2));
            }
        }
    }
    eprintln!("{name}: {} files, {:.1} MB, the best of {rounds} rounds in turns", files.len(), bytes as f64 / 1e6);
    for ((label, _), tiers) in forms.iter().zip(&best) {
        let show = |(ns, ins, cyc): (f64, u64, u64)| format!("{:6.2} ms CPU, {:6.1}M instructions, {:6.1}M cycles, IPC {:.2}", ns / 1e6, ins as f64 / 1e6, cyc as f64 / 1e6, ins as f64 / cyc as f64);
        eprintln!("  optimizer {label:>6}: tier 1 {}; tier 2 {}", show(tiers[0]), show(tiers[1]));
    }
}

/// Step 15 on real files with comments: JSONTestSuite's valid files and the corpus's smaller
/// files, each with comments put between tokens (after a leaf outside a string, after a string,
/// or at the start), where tree-sitter-json's own tree says the tokens end. Step 15's nodes equal
/// tree-sitter-json's on every one, comments placed where it places them. Comments holding a
/// quote, which `jsonc` sends to tree-sitter by design, are counted apart.
#[test]
#[ignore = "needs JSONTestSuite and a corpus: run by name with RERAIL_JSON_SUITE and RERAIL_JSON_CORPUS"]
fn jsonc_comments_equal_tree_sitter() {
    let jsonc = form("jsonc");
    let mut paths: Vec<PathBuf> = files_in("RERAIL_JSON_SUITE").into_iter().filter(|p| base(p).starts_with("y_")).collect();
    paths.extend(files_in("RERAIL_JSON_CORPUS").into_iter().filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 100_000)));
    const CLEAN: [(&str, bool); 8] = [("// c", true), ("//", true), ("// a, [1] é\r", true), ("/* c */", false), ("/**/", false), ("/***/", false), ("/* a * b / c */", false), ("/*\n{\n*/", false)];
    const QUOTED: [(&str, bool); 2] = [("// \"x", true), ("/* \" */", false)];
    let mut rng = rerail_cpu_stream::ladder::Rng(0x5eed_c0de);
    let (mut equal, mut quoted_refused, mut ts_errors, mut problems) = (0usize, 0usize, 0usize, Vec::new());
    for path in &paths {
        let bytes = std::fs::read(path).expect("reads");
        // Where a comment may go: the start, after each token outside a string (a delimiter
        // too, though the tapes leave it out), after each string.
        let Some(tree) = tree_sitter_json_with(&bytes, true) else { continue };
        let mut parent: Vec<usize> = vec![usize::MAX; tree.len()];
        let mut open: Vec<usize> = Vec::new();
        for (i, up) in parent.iter_mut().enumerate() {
            while open.last().is_some_and(|&o| i >= o + tree[o].4) {
                open.pop();
            }
            if let Some(&o) = open.last() {
                *up = o;
            }
            open.push(i);
        }
        let mut gaps: Vec<usize> = vec![0];
        for (i, n) in tree.iter().enumerate() {
            let in_string = parent[i] != usize::MAX && tree[parent[i]].0 == "string";
            if (n.4 == 1 && !in_string) || n.0 == "string" {
                gaps.push(n.3);
            }
        }
        gaps.sort_unstable();
        gaps.dedup();
        for variant in 0..6 {
            let quoted = variant == 5;
            let mut at: Vec<usize> = (0..1 + rng.below(4)).map(|_| *rng.pick(&gaps)).collect();
            at.sort_unstable();
            let mut doc = Vec::new();
            let mut from = 0;
            for &p in &at {
                doc.extend(&bytes[from..p]);
                let (c, line) = if quoted { *rng.pick(&QUOTED) } else { *rng.pick(&CLEAN) };
                doc.push(b' ');
                doc.extend(c.as_bytes());
                doc.push(if line { b'\n' } else { b' ' });
                from = p;
            }
            doc.extend(&bytes[from..]);
            let ts = tree_sitter_json(&doc);
            let got = ours(&jsonc, &doc).map(|n| named(&jsonc, &n));
            match (&got, &ts) {
                (_, None) => ts_errors += 1,
                (None, Some(_)) if quoted => quoted_refused += 1,
                (Some(g), Some(t)) if g == t => equal += 1,
                _ => {
                    if problems.len() < 5 {
                        problems.push(format!("{} variant {variant}: {:?}\n  ours {:?}\n  tree-sitter {:?}", base(path), String::from_utf8_lossy(&doc[..doc.len().min(300)]), got.as_ref().map(|g| &g[..g.len().min(12)]), ts.as_ref().map(|t| &t[..t.len().min(12)])));
                    } else {
                        problems.push(String::new());
                    }
                }
            }
        }
    }
    eprintln!("{} files: {equal} with comments equal to tree-sitter-json; {quoted_refused} with a quote in a comment sent to tree-sitter; {ts_errors} tree-sitter finds an error in", paths.len());
    assert!(problems.is_empty(), "{} differ:\n{}", problems.len(), problems.join("\n"));
}

/// Each form's shape (`RERAIL_FORMS`: other grammars' files), printed: tier 1's operations by kind, its carried state, its generated
/// loop body by opcode, and tier 2's words by opcode, with the machine code's sizes.
#[test]
#[ignore = "a measurement: run by name with --nocapture"]
fn json_shapes() {
    // `RERAIL_FORMS`: other Bitlex files to measure, as paths, in place of the two steps.
    let forms: Vec<(String, Compiled)> = match std::env::var("RERAIL_FORMS") {
        Ok(list) => list
            .split(',')
            .map(|p| {
                let name = Path::new(p).file_stem().expect("a name").to_string_lossy().into_owned();
                let c = compile_step(&name, &std::fs::read_to_string(p).expect("reads"));
                (name, c)
            })
            .collect(),
        Err(_) => ["json-rfc", "jsonc"].iter().map(|&n| (n.to_owned(), form(n))).collect(),
    };
    for (name, c) in &forms {
        let name = name.as_str();
        let p = &c.program.block_programs[0];
        let mut ops: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for op in &p.ops {
            let k = format!("{op:?}");
            *ops.entry(k.split(['(', ' ', '{']).next().unwrap_or("").to_owned()).or_default() += 1;
        }
        let body = clif_text::body_of(&c.parser.tier1.clif);
        let mut clif: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for l in &body {
            *clif.entry(clif_text::opcode(l).to_owned()).or_default() += 1;
        }
        // The disassembly's instruction lines: indented, not an unwind note, not a label.
        let machine = |asm: &str| asm.lines().filter(|l| l.starts_with("  ") && !l.trim_start().starts_with("unwind") && !l.trim_end().ends_with(':')).count();
        let mut words: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for w in &c.native.structural {
            let k = format!("{:?}", w.operation);
            *words.entry(k.split(['(', ' ', '{']).next().unwrap_or("").to_owned()).or_default() += 1;
        }
        eprintln!("== {name}");
        eprintln!("tier 1: {} operations {ops:?}", p.ops.len());
        eprintln!("  carried words {}, state bits {}, outputs {}, delay {}", clif_text::carried_words(p), clif_text::state_bits(p), p.outputs.len(), p.delay);
        eprintln!("  loop body: {} CLIF instructions {clif:?}", body.len());
        // Each block of the hot loop: the full body's head (the ASCII test), its two bodies, the
        // sinks after them, the flattening.
        let blocks: Vec<String> = clif_text::hot_loop(&c.parser.tier1.clif)
            .iter()
            .map(|(b, l)| {
                let masks = l.iter().filter(|x| clif_text::opcode(x) == "iadd_pairwise").count() / 4;
                format!("{b} {} ({masks} masks)", l.len())
            })
            .collect();
        eprintln!("  hot loop blocks: {}; carried words {}", blocks.join(", "), clif_text::carried_params(&c.parser.tier1.clif) - clif_text::sink_params(&c.native.block_programs[0]));
        let state = native_cpu_block::State::of(&c.native.block_programs[0]);
        let insts = &c.native.block_programs[0].insts;
        let fields: Vec<String> = state
            .fields()
            .iter()
            .map(|&(at, bits, word)| {
                let kind = if state.plan.lanes[at] { "lane" } else if state.plan.region_of[at].is_some() { "region" } else { "word" };
                format!("w{word}:%{at}={:?}/{bits}/{kind}", insts[at])
            })
            .collect();
        eprintln!("  carried state: {} words; {}", state.words, fields.join(" "));
        let plan = state.plan;
        for r in &plan.regions {
            let insts = &c.native.block_programs[0].insts;
            let gate = insts[r.gate];
            let of: Vec<String> = gate.operands().iter().map(|&o| format!("%{o} = {:?}", insts[o])).collect();
            let pre = r.pre.map_or(String::new(), |p| format!(", tested first on %{p}"));
            eprintln!("  gated region: gate %{} = {gate:?} ({}){pre}, {} values zero without it, {} skipped", r.gate, of.join(", "), r.zero.len(), r.members.len());
        }
        eprintln!("  machine code: {} instructions in the function", machine(&c.parser.tier1.disassembly));
        // The hot loop as `json_loop_keeps_values_in_registers` counts it, to set beside
        // simdjson's (`scripts/dev/simdjson-stage1-loop`); `RERAIL_DUMP_LOOP`: a directory for it.
        let hot = clif_text::machine_loop(&c.parser.tier1.loops_disassembly());
        eprintln!("  hot loop: {} instructions, {} stack loads or stores", hot.len(), clif_text::loop_costs(&hot).0);
        if let Ok(dir) = std::env::var("RERAIL_DUMP_LOOP") {
            std::fs::write(format!("{dir}/{name}.s"), hot.join("\n")).expect("writes");
        }
        eprintln!("tier 2: {} words {words:?}", c.native.structural.len());
        eprintln!("  switches {}, strings {}", c.native.switches.len(), c.native.strings.len());
        eprintln!("  machine code: {} instructions in the function", machine(&c.parser.tier2.disassembly));
        match std::env::var("RERAIL_DUMP").as_deref() {
            Ok("clif") => eprintln!("{}", c.parser.tier1.clif),
            Ok("ops") => {
                // Each value: on the byte vectors, made a mask, its region, its readers.
                let insts = &c.native.block_programs[0].insts;
                for (at, inst) in insts.iter().enumerate() {
                    let users: Vec<String> = (0..insts.len()).filter(|&u| insts[u].operands().contains(&at)).map(|u| format!("%{u}")).collect();
                    eprintln!("%{at} = {inst:?} lane={} mask={} full={:?} region={:?} users={}", plan.lanes[at], plan.mask[at], plan.full[at], plan.region_of[at], users.join(","));
                }
            }
            Ok(_) => eprintln!("{}", c.parser.tier1.disassembly),
            Err(_) => {}
        }
    }
}

/// Tier 1 on the byte vectors (`native_cpu_lanes`) equals the block interpreter on files that
/// mix ASCII and other bytes: UTF-8 that is valid, cut short or out of range, across block
/// edges, in the 16 bytes before an ASCII block (the ASCII test reads them), and at a file's
/// first byte, where the bytes before are no file's. One file at a time, and all in one batch.
#[test]
fn json_tier1_vectors_equal_the_interpreter() {
    let pieces: [&[u8]; 16] = [
        b"{\"a\": [1, 2.5e3, true]}", b" ", b"\"", b"\\", b"x", b"\\u00e9", b"\xc3\xa9", b"\xe2\x82\xac", b"\xf0\x9f\x98\x80",
        b"\x80", b"\xc3", b"\xed\xa0\x80", b"\xf5\x80\x80\x80", b"\xef\xbb\xbf", b"\x00", b"\\ud83d\\ude00",
    ];
    for name in ["json-rfc", "jsonc"] {
        let c = form(name);
        let mut rng = rerail_cpu_stream::ladder::Rng(0x9e37_79b9_7f4a_7c15);
        let mut files: Vec<Vec<u8>> = Vec::new();
        for _ in 0..300 {
            let mut f = Vec::new();
            let len = 1 + rng.below(300) as usize;
            while f.len() < len {
                // Mostly ASCII, so that some blocks are all ASCII and others not.
                let p = if rng.below(4) == 0 { *rng.pick(&pieces) } else { pieces[..2][rng.below(2) as usize] };
                f.extend_from_slice(p);
            }
            files.push(f);
        }
        // A byte order mark, then a string or a comment that runs into the next block: the full
        // body takes the file's start check as zero (`Plan::full`), and the edge part's carries
        // are the next block's (0383).
        for at in 0..80 {
            let escapes = [&b"\""[..], &[b'\\'; 70], b"\" 1"].concat();
            let bodies: [&[u8]; 4] = [b"\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\" 1", b"/* xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx */ 1", &escapes, b"1.5e3"];
            for body in bodies {
                let mut f = b"\xef\xbb\xbf".to_vec();
                f.extend(std::iter::repeat_n(b' ', at));
                f.extend_from_slice(body);
                files.push(f);
            }
        }
        // One non-ASCII sequence at each offset around the first two block edges.
        for at in 0..140 {
            for seq in [&b"\xe2\x82\xac"[..], b"\x80", b"\xf0\x9f\x98\x80"] {
                let mut f = vec![b' '; 200];
                f[at..at + seq.len()].copy_from_slice(seq);
                files.push(f);
            }
        }
        let block = &c.program.block_programs[0];
        let made = rerail_compiler::block_native::BlockNative::lower(block).cpu_unstored();
        for f in &files {
            assert_eq!(c.parser.tier1.extract(f).without(&made), block.run_files(&[f]).remove(0).without(&made), "{name}: {:?}", String::from_utf8_lossy(f));
        }
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let without = |runs: Vec<rerail_compiler::block::BlockRun>| runs.iter().map(|r| r.without(&made)).collect::<Vec<_>>();
        assert_eq!(without(c.parser.tier1.run_files(&refs)), without(block.run_files(&refs)), "{name}: the batch");
        assert!(c.parser.tier1.ascii_branch, "{name}: no ASCII test");
    }
}

/// The values the full body gets wrong on an edge block (`native_cpu_edge`, 0383): JSON's are
/// its checks at the stream's ends and what reads them, word operations all, a third of the
/// body's or fewer, and found in one pass of the analysis. The byte order mark is the host's
/// (0389): stripped before the passes, as simdjson strips it, the program the passes run has no
/// check of it, and no value differs for it through block 0.
#[test]
fn json_edge_values_are_its_ends_checks() {
    for name in ["json-rfc", "jsonc"] {
        let c = form(name);
        let (prefix, stripped) = native_cpu_block::host_stripped(&c.native.block_programs[0]).expect("the host strips the byte order mark");
        assert_eq!(prefix, b"\xef\xbb\xbf", "{name}: the prefix");
        assert_eq!(c.parser.tier1.prefix, prefix, "{name}: tier 1 strips it");
        let native = &stripped;
        let t = std::time::Instant::now();
        let plan = native_cpu_block::State::of(native).plan;
        let found = native_cpu_edge::edge_values(native, &plan.lanes, &plan.full).expect("bounded");
        let took = t.elapsed();
        println!("{name}: outputs needing no cut {:?}", found.cut_free);
        let edge = found.differs;
        let at: Vec<usize> = (0..edge.len()).filter(|&x| edge[x]).collect();
        println!("{name}: {} of {} values differ on an edge block, found in {took:?}: {at:?}", at.len(), edge.len());
        assert!(at.iter().all(|&x| !matches!(native.insts[x], block_native::BlockInst::EqByte(_) | block_native::BlockInst::Class { .. } | block_native::BlockInst::Pair { .. })), "{name}: a byte test differs");
        // A ratchet on the values the edge part makes again, apart from the body's size: lower
        // it as the compiler improves. (It was a third of the body's; the body shrank as the
        // number and `\u` checks went to tier 2, 0398 and 0399, and the edge's count less.)
        let most = if name == "json-rfc" { EDGE_VALUES_14 } else { EDGE_VALUES_15 };
        assert!(at.len() <= most, "{name}: {} of {} differ (budget {most})", at.len(), edge.len());
    }
}

/// The values an edge block differs in (a ratchet): json-rfc's 90 before 0389, 47 after, 41 with
/// its number and `\u` checks tier 2's.
const EDGE_VALUES_14: usize = 41;
/// jsonc's: 63 before 0389, 64 after, 56 with its checks tier 2's.
const EDGE_VALUES_15: usize = 56;

/// JSON's tier 1 has no general body (0383): its full body runs on the edge blocks too, with
/// its edge part, as simdjson's stage 1 runs one body and a `finish()`.
#[test]
fn json_tier1_has_no_general_body() {
    for name in ["json-rfc", "jsonc"] {
        let c = form(name);
        for (k, pass) in c.parser.tier1.passes.iter().enumerate().filter(|(_, p)| !p.back) {
            assert!(!pass.general, "{name}: tier 1's pass {k} calls a general body");
        }
    }
}

/// The full body's classes as simdjson's (0375): each place they are made in (the body's own
/// part, a gated region, the ASCII split's inside) has at most one pair of nibble tables, its
/// classes all in it, and none looked up again in a place inside the own part.
#[test]
fn json_classes_are_one_pair_a_place() {
    for name in ["json-rfc", "jsonc"] {
        let c = form(name);
        let native = &c.native.block_programs[0];
        let state = native_cpu_block::State::of(native);
        let pack = native_cpu_block::full_pack(native, &state);
        println!("{name}: tables by place {:?}", pack.places);
        assert!(pack.places.first().is_some_and(|&(p, t)| p == 0 && t == 1), "{name}: the body's own part has not one pair: {:?}", pack.places);
        assert!(pack.places.iter().all(|&(_, t)| t <= 1), "{name}: a place with more than one pair: {:?}", pack.places);
        let total: usize = pack.places.iter().map(|p| p.1).sum();
        assert_eq!(total, pack.packing.tables.len());
    }
}

/// The gated regions (`native_cpu_lanes::Region`): skipped on a block without their gate's bytes
/// and without their carried bits, so a number, an escape or a keyword must come out the same
/// wherever it falls, a block edge inside it, or spanning a block, or ending one.
#[test]
fn json_gated_regions_equal_the_interpreter() {
    let tokens: [&[u8]; 14] = [
        b"1", b"-12.5e+30", b"0", b"01", b"1.", b"-", b"2e", b"123456789012345678901234567890", b"\"\\u00e9\"", b"\"\\ud83d\\ude00\"",
        b"\"\\u12\"", b"true", b"nul", b"[1,2]",
    ];
    for name in ["json-rfc", "jsonc"] {
        let c = form(name);
        // Each region behind its branch in the full body (one: the ASCII test is a branch in it,
        // 0364), none computed in place. A region with a test before its gate's has two.
        // The plan of the program the CPU compiles: its form (`cpu_form`), whose outputs the
        // record pass makes are zero, so values only they read are a region's (decision 0428:
        // with the lone low surrogate tier 2's, the forms' plans differ).
        let plan = native_cpu_block::State::of(&c.native.block_programs[0].cpu_form()).plan;
        assert!(!plan.regions.is_empty(), "{name}: no gated region");
        let tests: usize = plan.regions.iter().map(|r| 1 + usize::from(r.pre.is_some())).sum();
        assert_eq!(c.parser.tier1.gate_branches, tests, "{name}: a region without its branch, or made twice");
        let block = &c.program.block_programs[0];
        let made = rerail_compiler::block_native::BlockNative::lower(block).cpu_unstored();
        let mut files: Vec<Vec<u8>> = Vec::new();
        for t in tokens {
            for at in 0..140 {
                // In a file of spaces, and at the file's end.
                let mut f = vec![b' '; 200];
                f[at..at + t.len()].copy_from_slice(t);
                files.push(f[..at + t.len()].to_vec());
                files.push(f);
            }
            // One file of the token over and over: every block holds one, or a part.
            files.push(t.iter().chain(b" ").copied().cycle().take(400).collect());
        }
        for f in &files {
            assert_eq!(c.parser.tier1.extract(f).without(&made), block.run_files(&[f]).remove(0).without(&made), "{name}: {:?}", String::from_utf8_lossy(f));
        }
    }
}


/// Why the JSONC form refuses each file of `RERAIL_JSON_FILES` (a path a line): each reason
/// with its count and a file. A probe: run by hand.
#[test]
#[ignore = "a probe: run by hand with RERAIL_JSON_FILES"]
fn jsonc_refusals_by_reason() {
    let c = form("jsonc");
    let list = std::fs::read_to_string(std::env::var("RERAIL_JSON_FILES").expect("set RERAIL_JSON_FILES")).expect("reads");
    let mut reasons: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if let Err(e) = c.parser.image(&bytes) {
            let r = reasons.entry(format!("{e:?}")).or_insert((0, path.to_owned()));
            r.0 += 1;
        }
    }
    for (reason, (n, path)) in &reasons {
        eprintln!("{n:5} {reason}: {path}");
    }
    if let Ok(out) = std::env::var("RERAIL_DISASM") {
        std::fs::write(out, rerail_compiler::rir_asm::disassemble(&c.program)).expect("writes");
    }
}

/// JSON's parser as Rust in simdjson's structure (`rerail_json_aot`): tier 1 and tier 2 as the Bitlex compiler writes
/// them from the grammar, tier 1 from the lexer's statements (`bitlex_lex_rust`, stage 1) and tier 2 from the tree
/// rules in one loop (`bitlex_rust::tier2_loop`), never from tier 2's native IR (decision 0551).
fn json_rust() -> (String, String) {
    let source = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../json_parser/json-simdjson.bitlex")).expect("reads");
    let stmts = ::bitlex::syntax::parse(&source).expect("parses");
    let ids = rerail_cpu_stream::ladder::Ids::default();
    let kind = |n: &str| Some(ids.of(n));
    let field = |n: &str| Some(ids.of(&format!("field {n}")));
    let symbols = rerail_bitlex::bitlex_masks::Symbols { kind: &kind, field: &field };
    let c = rerail_bitlex::bitlex_masks::compile_shaped(&stmts, &[rerail_bitlex::bitlex_masks::FALLBACK], Some(&symbols), rerail_bitlex::bitlex_masks::Shape::Auto).expect("compiles");
    let tier1 = c.lexer_rust.expect("a tree section").unwrap_or_else(|e| panic!("tier 1 as Rust: {e}"));
    let tier2 = c.rust_loop.expect("a tree section").unwrap_or_else(|e| panic!("tier 2 as one loop: {e}"));
    (tier1, tier2)
}

/// `rerail_json_aot`'s `tier1.rs` and `tier2.rs` are the Bitlex compiler's output for the grammar (`json_rust`);
/// `RERAIL_BLESS=1` writes them again.
#[test]
fn json_rust_is_current() {
    let (tier1, tier2) = json_rust();
    for (file, rust) in [("tier1.rs", tier1), ("tier2.rs", tier2)] {
        let path = format!("{}/../json_aot/src/{file}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("RERAIL_BLESS").is_some() {
            std::fs::write(&path, &rust).expect("writes");
        }
        assert!(std::fs::read_to_string(&path).is_ok_and(|have| have == rust), "json_aot's {file} is not the compiler's: RERAIL_BLESS=1");
    }
    let tier2 = json_rust().1;
    // An empty object or array one leaf, at its call (decision 0553): at every call of each, a value's arm.
    for (opener, kind) in [("0x7b", "object"), ("0x5b", "array")] {
        // A call's arm: the depth's test, or first the root's closer's.
        let calls = tier2.matches(&format!("{opener} => {{\nif depth == DEPTH {{")).count() + tier2.matches(&format!("{opener} => {{\nif src[starts[count - 1]")).count();
        assert!(calls > 0 && tier2.matches(&format!("// `{kind}`, empty.")).count() == calls, "every call of {kind} peeks for its closer");
    }
    // A value's decision once a site of the grammar's, not once a word of the number's row: the root's, a pair's
    // value, and an array's element (its list `value (',' value)*` one place, decision 0583).
    assert_eq!(tier2.matches("// `object`, empty.").count(), 3, "a value's decision a site");
    // The root's closer checked before the walk, where its value is a container: once each.
    for b in ["0x7d", "0x5d"] {
        assert_eq!(tier2.matches(&format!("if src[starts[count - 1] as usize] != {b} {{")).count(), 1, "the closer {b} checked once");
    }
}

/// The generated parser's format 16 and 16z tapes are the JIT's, byte for byte, in a language whose table is the
/// one the generator made (`ENTRIES`); it refuses the files the JIT refuses.
#[test]
fn json_rusts_tapes_are_the_jits() {
    let jit = simdjson_form();
    let (_, tag) = jit.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), rerail_json_aot::WIDTHS, 1).expect("a language");
    let numbers = jit.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
    let mut inputs = tree_inputs();
    inputs.extend(deep_nests(1023..1026));
    // Empty containers (one leaf each, decision 0553), and files whose last record is not the root's closer.
    for f in [&b"[]"[..], b"{}", b"[[]]", b"[{}]", b"{\"a\":[]}", b"{\"a\":{}}", b"[[],{},[[]],{\"b\":[]}]", b"{\"a\":[],\"b\":{},\"c\":1}", b"[ ]", b"{ }", b" [] ", b"[1] 2", b"{} {}", b"[]]", b"{}}", b"[1]]", b"{\"a\":1}]", b"[1", b"{", b"[", b"[]["] {
        inputs.push(f.to_vec());
    }
    // UTF-8 in strings, two to four bytes a character, and each kind of invalid UTF-8 (tier 1's errors, which read the
    // class `high` where the grammar defines a mask of that name after them).
    for f in ["[\"é\"]", "{\"日本\": \"語\"}", "[\"😀 x 😀\", \"\\u00e9\"]", "\"ü\""] {
        inputs.push(f.as_bytes().to_vec());
    }
    for f in [&b"[\"\xc3\"]"[..], b"[\"\x80\"]", b"[\"\xc0\xaf\"]", b"[\"\xed\xa0\x80\"]", b"[\"\xf5\x80\x80\x80\"]", b"[\"\xe6\x97\"]", b"[\"a\"]\xc3"] {
        inputs.push(f.to_vec());
    }
    // `\u`'s digits and surrogates, tier 2's at the escape (decision 0599): pairs, lone ones each way, a high one before
    // another escape or another high, too few digits or not hex, an escaped backslash before a `u`, a pair split over two
    // strings, and a pair across a block's edge.
    for f in [
        r#"["\uD83D\uDE00"]"#, r#"["\ud83d\ude00"]"#, r#"["\uD83D"]"#, r#"["\uDE00"]"#, r#"["\uDE00\uD83D"]"#, r#"["\uD83D\n"]"#,
        r#"["\uD83Dx\uDE00"]"#, r#"["\uD83D\uD83D\uDE00"]"#, r#"["\u12"]"#, r#"["\u12G4"]"#, r#"["\\u12"]"#, r#"["\\uD83D\uDE00"]"#,
        r#"["\\uDE00"]"#, r#"["a\uD83D","\uDE00"]"#, r#"{"\u00e9": "\u0041\u00"}"#,
        r#"["\uD83D\uDE0g"]"#, r#"["\uD83D\uDE"]"#, r#"["\uD83D\uDE00\uDE00"]"#, r#"["\uD83D\uDE00\uD83D\uDE01x"]"#, r#"["\uD83D\uDBFF"]"#, r#"[1"a"]"#, r#"[true"a"]"#, r#"{"a":1"b"}"#, r#"[1 "a"]"#, r#"["a""b"]"#, r#"["\x"]"#, r#"["\'"]"#, r#"["\U0041"]"#, r#"["a\"#, r#"["\"#, r#"["\\"]"#, r#"["\/\b\f\n\r\t\""]"#,
    ] {
        inputs.push(f.as_bytes().to_vec());
    }
    for k in 50..70 {
        inputs.push(format!("[\"{}\\uD83D\\uDE00\"]", "x".repeat(k)).into_bytes());
        inputs.push(format!("[\"{}\\uD83D\\u0041\"]", "x".repeat(k)).into_bytes());
    }
    // After a byte order mark (the blocks start after it): alone, short, unclosed, a scalar, and across a block's edge.
    let bom = |b: &[u8]| [&[0xef, 0xbb, 0xbf][..], b].concat();
    for b in [&b""[..], b" ", b"[1]", b"\"a", b"7", b"[\"\xef\xbb\xbf\"]", b"\xef\xbb\xbf[]", b"{\"a\": [true, null]}"] {
        inputs.push(bom(b));
    }
    for k in 55..72 {
        inputs.push(bom(format!("[\"{}\", 12, \"b\\n\"]", "x".repeat(k)).as_bytes()));
        inputs.push(bom(format!("[{}1\"a\"]", " ".repeat(k)).as_bytes()));
        inputs.push(bom(format!("[\"{}", "x".repeat(k)).as_bytes()));
        inputs.push(format!("[\"{}", "x".repeat(k)).into_bytes());
        inputs.push(format!("[\"{}\"]", "x".repeat(k)).into_bytes());
    }
    let (mut same, mut refused) = (0, 0);
    for (k, f) in inputs.iter().enumerate() {
        let show = || String::from_utf8_lossy(&f[..f.len().min(60)]).into_owned();
        for z in [false, true] {
            let want = jit.parser.image16(f, &lang, &numbers, z).map(|w| w.extent().to_vec());
            let mut w = rerail_tape_format::f16::Writer::new(&lang, f, tag, z, true, f.len() / 2 + 16);
            let got = rerail_json_aot::parse(&rerail_json_aot::padded(f), f.len(), &mut w).map_err(|b| format!("refused at byte {b}")).and_then(|()| w.finish().map(|d| d.tape.extent().to_vec()));
            match (&want, &got) {
                (Ok(a), Ok(b)) => {
                    assert!(a == b, "file {k} ({:?}, 16z {z}): the Rust's tape is not the JIT's; the JIT's nodes {:?}", show(), ours(&jit, f));
                    same += 1;
                }
                (Err(_), Err(_)) => refused += 1,
                _ => panic!("file {k} ({:?}, 16z {z}): the JIT {want:?}, the Rust {got:?}", show()),
            }
        }
    }
    eprintln!("{same} tapes the JIT's, {refused} refused by both");
    assert!(same > 0 && refused > 0);
}

/// JSON's UTF-8 errors (`invalid UTF-8`, `UTF-8 cut short at the end`) are simdjson's `utf8_checker` in tier 1's Rust
/// (decision 0591): the lookup is proved the grammar's statements (`bitlex_pair_lookup`), and tier 1 holds its three
/// tables, the 2-and-3 test, the ASCII skip and the open-sequence test, and none of the UTF-8 classes' masks.
#[test]
fn json_utf8_is_simdjsons_lookup() {
    let source = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../json_parser/json-simdjson.bitlex")).expect("reads");
    let stmts = ::bitlex::syntax::parse(&source).expect("parses");
    let l = rerail_bitlex::bitlex_pair_lookup::lookup(&stmts, Some(&[0xef, 0xbb, 0xbf])).unwrap_or_else(|e| panic!("not the lookup form: {e}"));
    let whys: Vec<&str> = l.covers.iter().map(|(_, w)| w.as_str()).collect();
    assert_eq!(whys, ["invalid UTF-8", "UTF-8 cut short at the end"]);
    // simdjson's: the 2-and-3 test at 0xe0 and 0xf0, and a block's end open at 0xc0, 0xe0, 0xf0 from its last byte.
    assert_eq!(l.xor, Some((0xe0, 0xf0)));
    assert_eq!(l.open, [Some(0xc0), Some(0xe0), Some(0xf0)]);
    let (tier1, _) = json_rust();
    for part in ["static BYTE_1_HIGH", "static BYTE_1_LOW", "static BYTE_2_HIGH", "let must23 =", "if unsafe { S::to_bitmask(v) } == 0 {", "prev_incomplete = unsafe { S::gt_bits(v, max_value) };", "0xff, 0xef, 0xdf, 0xbf];"] {
        assert!(tier1.contains(part), "tier 1 has no `{part}`");
    }
    for class in ["cont", "lead3", "lead4", "e0", "ed", "f0", "f4", "below_a0", "from_a0", "below_90", "from_90", "not_cont", "lead", "c0c1", "f5ff", "third", "fourth"] {
        assert!(!tier1.contains(&format!("r_m_{class}[")) && !tier1.contains(&format!("v_m_{class} ")), "tier 1 still makes the mask `{class}`");
    }
}

/// Tier 2 in `walk`'s shape (page 40, fixes 8, 9 and 12; decisions 0599, 0602): each container's cycle one loop, its
/// opener taken in its arm, a pair closed where its value ends, a value's primitives one method called from each
/// place, a string with no escape written as its node and its content at once.
#[test]
fn json_tier2_is_walks_shape() {
    let (_, tier2) = json_rust();
    // Each container's list one labeled loop, its separator back to the element inside it.
    let loops: Vec<&str> = tier2.lines().filter(|l| l.starts_with("'l") && l.ends_with(": loop {")).collect();
    assert_eq!(loops.len(), 2, "an object's and an array's loop: {loops:?}");
    for l in &loops {
        let label = l.trim_end_matches(": loop {");
        assert!(tier2.contains(&format!("continue {label};")), "{label}: its separator goes back inside the loop");
    }
    // Its opener taken in the loop's arm (`ObjectBegin | ObjectKey`).
    assert_eq!(tier2.lines().filter(|l| l.strip_suffix(" => {").and_then(|p| p.split_once(" | ")).is_some_and(|(a, b)| a.parse::<usize>().is_ok() && b.parse::<usize>().is_ok())).count(), 2, "a begin and its loop one arm, each container");
    // The places a level returns to one arm, a loop (`walk`'s `scope_end`): an object's and an array's rest and the
    // document's end, a closer there going on inside it, not through the run's match.
    let ret = tier2.split(" => 'ret: loop {\n").nth(1).expect("the returns' loop").split("\n},\n").next().expect("its end");
    assert_eq!(tier2.matches("=> 'ret: loop {").count(), 1, "one returns' loop");
    assert_eq!(ret.matches("st = ret[depth];\ncontinue 'ret;").count(), 2, "an object's and an array's closer stay in it:\n{ret}");
    assert!(!ret.contains("st = ret[depth];\ncontinue 'run;"), "a closer in it through the run's match");
    // A separator's, an opener's and a `:`'s end kept by none (`walk`'s `take!()`): the record taken alone. A closer's
    // is its node's end.
    assert_eq!(tier2.matches("0x2c => {\ni += 1;\n").count(), 4, "each `,` taken alone");
    assert!(tier2.contains("0x3a => {\ni += 1;\n}"), "the `:` taken alone");
    assert_eq!(tier2.matches("e = s + 1;").count(), 4, "only the closers' ends");
    // No place that only closes a node and goes on: a pair is closed where its value ends.
    assert!(!tier2.contains("=> {\nclose(d, e as u64);\nst = "), "a place of a pair's close alone");
    // A value as `walk` decides it (decision 0602): at each of its three places (the root, a pair's value, an element)
    // a `match` on the byte with the containers' arms, and the rest one method's (`walk`'s `primitive`), its `_`. The
    // method is written once: the string's, the atoms' and the number's arms there alone.
    assert_eq!(tier2.matches("fn value_primitive<S").count(), 1, "the primitive written once");
    let calls: Vec<&str> = tier2.split("match peek(src, starts, i) {\n").skip(1).filter(|m| m.contains("value_primitive::<S>(")).collect();
    let places: Vec<&str> = tier2.split("let s = starts[i] as usize;\nmatch src[s] {\n0x7b => {").skip(1).collect();
    assert_eq!(places.len() + calls.len(), 3, "a value's decision at three places");
    assert_eq!(tier2.matches("_ => {\ne = unsafe { value_primitive::<S>(src, starts, d, &mut i, [").count(), 3, "each place's `_` the primitive");
    let primitive = tier2.split("fn value_primitive<S").nth(1).expect("the primitive").split("\nOk(e)\n}").next().expect("its body");
    for arm in ["0x22 => {\ne = unsafe { string::<S>(", "0x74 if src[s..s + 4] == *b\"true\"", "0x66 if src[s..s + 5] == *b\"false\"", "0x6e if src[s..s + 4] == *b\"null\"", "let end = number_end(src, s)?;\ne = token(d, &mut i, "] {
        assert!(primitive.contains(arm), "`{arm}` the primitive's:\n{primitive}");
        assert_eq!(tier2.matches(arm).count(), 1, "`{arm}` written once");
    }
    assert!(!primitive.contains("0x7b") && !primitive.contains("0x5b"), "a container is the place's");
    assert_eq!(tier2.matches("fn number_end(").count(), 1, "a number's end written once");
    // No depth test at the root's calls (no level is open there, as `walk`'s document); one at each list's.
    let root = tier2.split("\n0 => {\n").nth(1).expect("the root's place").split("\n1 | 2 => {").next().expect("its end");
    assert!(!root.contains("depth == DEPTH"), "a depth test at the root:\n{root}");
    assert_eq!(tier2.matches("if depth == DEPTH {").count(), 4, "an object's and an array's call in each list");
    // A scalar's end as simdjson's: the byte after it structural or whitespace (`is_not_structural_or_whitespace`).
    assert!(tier2.contains("fn ends(b: u8) -> bool {\nmatches!(ROWS[usize::from(b)], 0 | 2)\n}"), "the end's test");
    assert!(tier2.contains("Some(j) if ends(src[s + j]) => Ok(s + j),") && tier2.contains("== *b\"true\" && ends(src[s + 4])"), "a number's and an atom's end");
    // A number at a digit or `-` alone (`walk`'s `*v - '0' < 10 || *v == '-'`), any other byte refused.
    assert!(primitive.contains("0x2d | 0x30..=0x39 => {\nlet end = number_end(src, s)?;"), "the number's arm its check's first bytes:\n{primitive}");
    // The number's check reads its bytes with no bound (the padding ends every number, as `number_end` reads), and the
    // exponent's sign in one test.
    let check = tier2.split("fn check_number(b: &[u8]) -> Option<usize> {\n").nth(1).expect("the check").split("\nSome(j)\n}").next().expect("its body");
    assert!(!check.contains("b.get(j)"), "a bound at each byte:\n{check}");
    assert!(check.contains("if matches!(b[j], 0x2b | 0x2d) {\nj += 1;\n}\n"), "the exponent's sign one test:\n{check}");
    assert!(!tier2.contains("i += 1;\ne = end;\nleaf("), "a token's leaf written out at a place");
    // The nodes stored through the walk's hold on the tape, a local of `run` (`Cursor`, the lean parser's `Walk`), not
    // the writer's ops on its state.
    assert_eq!(tier2.matches("let mut cursor = unsafe { Cursor::<Cells>::begin(state) };").count(), 1, "one cursor, the walk's");
    for op in ["w_open_with", "w_leaf_with", "w_close_with", "w_leaf_in_with", "d: *mut u8"] {
        assert!(!tier2.contains(op), "tier 2 still has `{op}`");
    }
    // A string read 32 bytes at a time on the backend chosen once (`run_best`), as `backslash_quote`.
    assert!(tier2.contains("x += 32;") && tier2.contains("S::marks32(") && tier2.contains("fn run_best("), "32 bytes a step, one dispatch");
    // A string with no escape: the node and its content at once.
    let string = tier2.split("fn string<S").nth(1).expect("string").split("\nOk(e)\n}").next().expect("its body");
    assert!(string.contains("leaf_in(d, "), "a plain string at once:\n{string}");
}

/// `\u`'s digits and surrogates are tested where tier 2 reads the escape (page 40, fix 10; decision 0599): tier 1 has
/// none of their classes or masks, and tier 2's `string` tests them at a `\u` alone.
#[test]
fn json_escape_checks_are_tier2s() {
    let (tier1, tier2) = json_rust();
    for m in ["unicode", "r_m_hex", "r_m_dee", "high_2", "low_2", "r_m_low", "lone"] {
        assert!(!tier1.contains(m), "tier 1 still has `{m}`");
    }
    let string = tier2.split("fn string<S").nth(1).expect("string").split("\nOk(e)\n}").next().expect("its body");
    let at = string.split("let y = p + 1;\n").nth(1).expect("the checks at an escape");
    // The escape's second byte decided once, as `escaped_string`'s `c`: the `u`'s checks, the other words, else refused.
    assert!(at.starts_with("// Its second byte decided once: the words', the checks at the gate's.\nmatch src[y] {\n0x75 => {\n"), "at a `\\u` alone:\n{at}");
    assert!(at.contains("\n0x22 | 0x5c | 0x2f | 0x62 | 0x66 | 0x6e | 0x72 | 0x74 => {}\n_ => return Err(p as u32),\n}"), "the other words, then refused:\n{at}");
    assert!(!string.contains("if !matches!(&src[p..p + 2]"), "the two bytes matched against the words");
    // No end bound: tier 1 refuses a file a string is open at the end of (`refuses_open_at_end`), as `walk` reads.
    assert!(!string.contains("x >= len"), "a bound at the file's end:\n{string}");
    for why in ["// \\u without four hex digits", "// a lone high surrogate", "// a lone low surrogate"] {
        assert!(at.contains(why), "{why}");
    }
    // As `escaped_string`: the four digits read as one number, the surrogates ranges of it; a high one's low half its
    // next escape's four digits, a number too; and that escape's checks skipped (`low_checked`), so a low one reached
    // otherwise is lone.
    for part in ["let cp = HEX_VALUE[usize::from(src[y + 1])] << 12 |", "if cp > 0xffff {", "let m2 = (0xd800..=0xdbff).contains(&cp);", "let m3 = (0xdc00..=0xdfff).contains(&cp);", "after2 = y + 6;", "if after2 != y {", "if m3 {\nreturn Err(y as u32);", "let n7 = HEX_VALUE[usize::from(src[y + 7])] << 12 |", "(0xdc00..=0xdfff).contains(&n7)"] {
        assert!(at.contains(part), "no `{part}`:\n{at}");
    }
    assert!(!string.contains("w2") && !at.contains("matches!(src[y + 1]"), "a digit's class tested, or the escapes kept:\n{at}");
}

/// Tier 1's masks are its block's locals (page 40, fix 11; decision 0599): no ring, the block before's word kept only
/// where a `prev` reads it, the file's bits and the byte order mark made once, no block past the file, the test at the
/// file's last byte (a string open there) once, after the pass, and the errors ORed and tested once.
#[test]
fn json_tier1_masks_are_block_locals() {
    let (tier1, _) = json_rust();
    let stage1 = tier1.split("unsafe fn stage1<").nth(1).expect("stage 1").split("\nrerail_simd8::dispatch!").next().expect("its body");
    assert!(!stage1.contains("RING") && !stage1.contains("r_m_"), "no ring:\n{stage1}");
    assert!(stage1.contains("    for t in 0..nw {\n"), "the file's blocks alone");
    assert!(!stage1.contains("NONE"), "no block past the file");
    assert_eq!(stage1.matches("src.starts_with(").count(), 1, "the byte order mark tested once");
    // No mask the file's bits (`lv`): no record or error holds at the padding's byte; the blocks start after a byte
    // order mark (`off`), as simdjson's stage 1 reads past it.
    assert!(!stage1.contains("lv"), "a mask of the file's bits:\n{stage1}");
    assert!(stage1.contains("let off = if src.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };\n    let nw = (n - off).div_ceil(64);"), "the blocks after the mark");
    // The words read past their block's code: `nonquote_scalar`, for the block before's (`follows`; a local of its
    // code there). The string open at the end is the toggle's carry out of the last block (simdjson's `finish`,
    // `prev_in_string != 0`): `in_string`'s word, as the grammar names the region's `f_str`, is its block's.
    let outer: Vec<&str> = stage1.lines().filter(|l| l.starts_with("    let mut v_") || l.starts_with("    let mut p_")).collect();
    assert_eq!(outer, ["    let mut v_m_nonquote_scalar = 0u64;"]);
    assert!(stage1.contains("            let p_m_nonquote_scalar = v_m_nonquote_scalar;\n"), "the block before's `nonquote_scalar`, a local");
    // The string and token masks are `index`'s (`json_scanner`, `json_string_scanner`): `scalar`, `nonquote_scalar`,
    // `follows`, `string_tail`, and the records `structural_start` itself (proved equal to the tokens' starts but where
    // an error refuses the file, `refused_equal`).
    for part in [
        "let v_m_scalar = !(v_m_op | v_m_ws);",
        "v_m_nonquote_scalar = v_m_scalar & !v_m_quote;",
        "let v_m_follows = (v_m_nonquote_scalar << 1 | p_m_nonquote_scalar >> 63);",
        "let v_m_string_tail = (v_m_in_string ^ v_m_q);",
        "let v_m_structural_start = (v_m_op | v_m_scalar & !v_m_follows) & !v_m_string_tail;",
        // The block before's records written at this block, the last after the pass, through one `write` (simdjson's
        // `bit_indexer::write`, `indexer.write(idx - 64, prev_structurals)` and `finish`).
        "unsafe { write_indexes(si, &mut count, ((off + 64 * q) as u32).wrapping_sub(64), prev_structurals) };\n            prev_structurals = v_m_structural_start;",
    ] {
        assert!(stage1.contains(part), "no `{part}`:\n{stage1}");
    }
    // A string right after a scalar is tier 2's, at the scalar's end (`run_end_covers`), as simdjson's stage 2.
    assert!(!stage1.contains("a string right after a scalar"), "tier 1 tests a scalar's dirty end");
    for name in ["v_m_word", "v_m_code", "v_m_wordbyte", "v_m_hidden", "v_m_o_str"] {
        assert!(!stage1.contains(name), "stage 1 still makes `{name}`");
    }
    let after = stage1.split("\n    }\n").nth(1).expect("after the pass");
    assert!(after.contains("// a string open at the end of the file: the toggle carried out of the file's last block, once (simdjson's `finish`).\n    refused |= c_m_in_string != 0;"), "the end's test after the pass:\n{after}");
    assert!(!stage1.contains("last_byte"), "no mask of the last byte");
    // The errors' bits ORed into one word, tested once after the pass (`index`'s `unescaped_chars_error`).
    assert!(stage1.contains("            errors |= v_t2;\n") && after.contains("    refused |= errors != 0;\n"), "the errors tested once");
    assert!(after.contains("    unsafe { write_indexes(si, &mut count, ((off + 64 * nw) as u32).wrapping_sub(64), prev_structurals) };"), "the last block's records after the pass");
    assert_eq!(tier1.matches("fn write_indexes(").count(), 1, "one write");
    for part in ["    if bits == 0 {\n        return;\n    }", "        if 6 < cnt {\n            one(*count + 6);"] {
        assert!(tier1.contains(part), "the write's `{part}`");
    }
}
