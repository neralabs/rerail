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

/// The language a format 16 source is generated for and its runs take (decision 0535): every key a direct run of the
/// grammar writes (`rust_stream::direct_entries`), then every other key the files' format 15 tapes hold, as the
/// engine's language holds its grammar's. One for every file, known before the Rust is written: its numbers are
/// literals there, and no node word of the grammar is left without one.
fn language_of(c: &Compiled, files: &[&[u8]]) -> rerail_tape_format::f16::Lang {
    let (widths, tag) = c.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    let mut keys: Vec<rerail_tape_format::tape::Entry> = rerail_rust_stream::rust_stream::direct_entries(&c.native, widths, tag).unwrap_or_default();
    for f in files {
        let Ok(image) = c.parser.image(f) else { continue };
        let (_, table) = rerail_tape_format::tape::f15_columns(&image).expect("a format 15 tape");
        for e in table.entries() {
            if !keys.contains(e) {
                keys.push(e.clone());
            }
        }
    }
    rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(keys), [1, 1, 1], 1).expect("a language")
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

/// `src` with each line's indent and its trailing spaces left out.
fn flat(src: &str) -> String {
    src.lines().map(str::trim).collect::<Vec<_>>().join("\n") + "\n"
}

/// Each dispatch's arm (`'X: loop { match d { … } }`, its top level) that is only `d = L::M;
/// continue 'X;`, back into its own loop, in generated Rust whose comments hold no braces.
#[cfg(target_arch = "aarch64")]
fn dispatch_resets(src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().map(str::trim).filter(|l| !l.starts_with("//")).collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let Some(b) = l.strip_prefix('\'').and_then(|r| r.strip_suffix(": loop {")) else { continue };
        let Some(m) = lines[i + 1..].iter().position(|l| !(l.starts_with('\'') && l.ends_with(": {"))).map(|k| i + 1 + k).filter(|&k| lines[k] == "match d {") else { continue };
        let own = format!("continue '{b}; }}");
        let mut depth = 1i64;
        for l in &lines[m + 1..] {
            let arm = l.split_once(" => { d = ").filter(|(a, _)| a.starts_with("L::"));
            if depth == 1 && arm.is_some_and(|(_, rest)| rest.split_once("; ").is_some_and(|(n, tail)| n.starts_with("L::") && tail == own)) {
                out.push(format!("'{b}: {l}"));
            }
            depth += l.matches('{').count() as i64 - l.matches('}').count() as i64;
            if depth <= 0 {
                break;
            }
        }
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
    // the writer's stack is reserved for (`rust_stream::open_bound`, decision 0537), at their most; past it, refused.
    inputs.extend(deep_nests(1022..1028));
    inputs.push(format!("{{{}}}", (0..1500).map(|k| format!("\"k{k}\": [{k}, \"v\\n\"]")).collect::<Vec<_>>().join(",")).into_bytes());
    inputs.push(format!("[{}1,{}\"{}\", [{}]]", " ".repeat(300), "\n".repeat(400), "s".repeat(700), vec!["null"; 300].join(", ")).into_bytes());
    inputs.push(format!("[{}]", (0..1100).map(|k| if k % 97 == 0 { format!("{}{k}", " ".repeat(260)) } else { k.to_string() }).collect::<Vec<_>>().join(",")).into_bytes());
    if std::env::var("RERAIL_JSON_CORPUS").is_ok() {
        inputs.extend(files_in("RERAIL_JSON_CORPUS").iter().map(|p| std::fs::read(p).expect("reads")));
    }
    inputs
}

/// The trees test's form: tier 2 as `rust_stream` writes it, compiled by rustc and run on the JIT's
/// harness in place of Cranelift's code, gives the JIT's trees, file for file (page 40: the
/// comparison with simdjson is made on this Rust).
#[cfg(target_arch = "aarch64")]
#[test]
fn simdjson_forms_rust_trees_are_the_jits() {
    let (jit, mut aot) = (simdjson_form(), simdjson_form());
    let native = rerail_compiler::native_ir::lower(&aot.program, 0, 0).expect("lowers");
    let (widths, tag) = aot.parser.tier2.widths_tag();
    let source = rerail_rust_stream::rust_stream::rust_source_at(&native, widths, tag).expect("tier 2 as Rust");
    // Its code read line by line, each line's indent left out (`flat`): the tests read the code, not its layout.
    let src = flat(&source);
    // A number is read once, as `parse_number` reads it: each arm of a number's first bytes calls one method, written
    // once (`number`), which makes its end and its check from one read of its bytes (`run_R_check_C`).
    let arms: Vec<&str> = src.split("0x2d | 0x30..=0x39 => {\n").skip(1).collect();
    assert!(!arms.is_empty(), "the numbers' arms");
    for arm in &arms {
        let first = arm.lines().nth(1).unwrap_or("");
        assert!(first.trim() == "s.number(st, t)?;", "a number's arm calls the number's method: {first}");
    }
    let number = src.split("fn number(&mut self, st: u32, t: &mut T) -> R {").nth(1).expect("the number's method");
    assert!(src.matches("fn number(").count() == 1 && number.lines().take(3).any(|l| l.trim().starts_with("match s.run_") && l.contains("_check_")), "the number read once, in one method");
    // Each word written once, a method each arm calls (decision 0518): one string loop (`string`, as the lean
    // parser's), one atom test (`atom`), called from each place a value is read.
    // The string is given the opener's place, as the lean parser's `string(v)`: no field the opener's arm sets.
    for m in ["fn string(&mut self, mut at: u32, t: &mut T) -> R {", "fn atom(&self, st: u32) -> u32 {"] {
        assert_eq!(src.matches(m).count(), 1, "one method: {m}");
    }
    assert!(src.matches("s.string(t.start + 1, t)?;").count() >= 3 && src.matches("s.atom(st)").count() >= 2, "the methods called from each place");
    assert!(!src.contains("part_at"), "the place a string's next part is at is the method's own");
    assert_eq!(src.matches("s.part_stop(").count(), 1, "one string loop");
    // One dispatch a value, as `walk_document`'s `switch (*value)`: no variable saying which
    // place a loop was entered at (the array's first value goes to the one value switch), and a
    // switch on the kind only where a call's return switches on its token again (`again`).
    // One function, as `walk_document`: the nesting on a stack of its own (`s.ret`, as
    // `is_array[depth]`), not the machine's; the loops a value is entered at from an array, an
    // object and the document are dispatched (`match d`), as simdjson's `goto`s into them.
    // A rule's function is named by its rule (`rust_stream`'s `function`): counted by its signature.
    assert!(src.matches("/// The rule entered at word ").count() == 1 && src.contains("ret = s.taken_ret();"), "one function, an explicit stack");
    // A return as `scope_end`'s (`rust_stream::root_number`, decision 0558): the document's where the depth is 0,
    // else the container's two places by one test of its number; no match, no refusal of a number, no number kept
    // by the document's own calls.
    let two_ways = |what: &str, code: &str| {
        assert!(code.contains("s.depth -= 1;\nif s.depth == 0 {") && code.contains("} else {\nif ret == 1 {") && !code.contains("match ret {") && !code.contains("why::RETURN)") && !code.contains("s.keep_ret(0)"), "format {what}: a return two ways");
    };
    two_ways("15", &src);
    // The token's state the one rule's local, as `walk`'s `next`, `v` and `end`: not the run's fields.
    let state = src.split("\nstruct S {").nth(1).and_then(|s| s.split("\n}\n").next()).expect("the run's state");
    assert!(src.contains("(s: &mut S) -> Result<bool, u32> {\nlet t = &mut T::NEW;") && !state.contains("start: u32") && !state.contains("rec_end") && !state.contains("last_end"), "the token's state a local of the rule");
    // One stack, the nodes': a call's number kept in its container's row (`keep_ret`), read back
    // by the `CLOSE` before its `RETURN`; no call stack of its own, and no `CLOSE` tests for a
    // node open (`OneFn::stacked`). The depth counts as simdjson's, for its limit alone.
    assert!(!src.contains("s.ret[") && src.contains("s.keep_ret(") && !src.contains("close::<true, false>") && !src.contains("close::<false, false>"), "one stack");
    let again: std::collections::BTreeSet<usize> = native
        .structural
        .iter()
        .filter_map(|w| if let rerail_compiler::native_ir::StructuralOperation::Push { target, .. } = w.operation { Some(usize::from(target) + 1) } else { None })
        .filter(|&t| matches!(native.structural.get(t).map(|w| w.operation), Some(rerail_compiler::native_ir::StructuralOperation::Switch { .. })))
        // A call's one after is a return's place only where a `RETURN` gives `again`.
        .filter(|_| native.structural.iter().any(|w| matches!(w.operation, rerail_compiler::native_ir::StructuralOperation::Return { again: true })))
        .collect();
    let kind_paths = src.matches("if t.rec_end == 0 && !t.at_end {").count() + src.matches("if t.rec_end == 0 && !(t.start == s.len) {").count();
    assert_eq!(kind_paths, again.len(), "the kind path only where a call returns: {again:?}");
    // The `\u` checks only after a `u`, as simdjson's `\u` work: every call of them in the arm of
    // the escape `\u`; every other escape goes past them.
    let words = &aot.program.block_programs[0].records.as_ref().expect("records").words;
    let u = words.iter().position(|w| w.as_slice() == b"\\u").expect("the escape \\u") + 1;
    // The word by its name (`mod w`), else its number.
    let u = src.lines().skip_while(|l| *l != "mod w {").find_map(|l| l.strip_prefix("pub const ").and_then(|r| r.strip_suffix(&format!(": u32 = {u};")))).map_or_else(|| u.to_string(), |n| format!("w::{n}"));
    let calls = src.matches("s.check_1(t)?;").count();
    assert!(calls > 0 && src.matches(&format!("{u} => {{\ns.check_1(t)?;")).count() == calls, "the \\u checks in the \\u arm alone");
    // The end tested only where it is taken: `TOKEN` gives the file's length as the start past
    // the last token, and the value switches refuse its byte, as simdjson's switch the index past
    // its last. No `at_end` field is read.
    assert!(!src.contains("t.at_end") && !src.contains("if (t.start == s.len) { return refuse("), "no end test where the switch refuses the end");
    // No record field written that no word reads: no `live` (no marks), no `rec_end = 0` (no
    // kind path), and the code only where the escapes' word `match` reads it; the atoms write
    // their leaves with their kinds, as simdjson's `case 't'` its tape word.
    let code_writes: Vec<&str> = src.lines().filter(|l| l.starts_with("t.code = ")).collect();
    assert!(code_writes.iter().all(|l| l.starts_with("t.code = S::CODES2[")) && !code_writes.is_empty(), "code written where no word reads it: {code_writes:?}");
    assert!(!src.contains("live =") && !src.contains("rec_end = 0;") && !src.contains("s.word_kind()") && !src.contains("rec_end == 0"), "a field written that no word reads");
    // A string as `copy_and_find`: 32 bytes a step, the stop's kind from the masks (no byte read
    // again), a content part then its stop in the same pass, and no test of a one-byte token's
    // length after it is set.
    let parts = src.matches("let (i, closer) = s.part_stop(at)?;").count();
    assert!(parts > 0 && src.contains("p += 32;") && !src.contains("matches!(s.byte(i)") && src.matches("}\nif closer {").count() == parts && !src.contains("!= t.rec_end"), "a string's stops as copy_and_find's");
    // Each node's code written as it is made (`code_now`), no commit pass over the nodes after.
    let made = src.matches("s.open(").count() + src.matches("s.leaf::<").count();
    assert!(!src.contains("self.commit(") && src.matches("s.code_now(").count() == made, "each node's code as it is made");
    // No dispatch's arm only sets `d` again for its own loop (`Shape::threaded`).
    // The dispatches' values are named, as the lean parser's `Label`s: each the place it goes on to.
    let numbered: Vec<&str> = src.lines().map(str::trim).filter(|l| l.split(['{', ';']).any(|st| st.trim().strip_prefix("d = ").is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit())))).collect();
    assert!(numbered.is_empty() && src.contains("enum L {") && src.contains("match d {"), "a dispatch's value a number: {numbered:?}");
    // One dispatch, as the lean parser's one `match label`: the loops within it entered once each, a jump into
    // another's header through the dispatch (`Shape::threaded`).
    assert_eq!(src.matches("match d {").count(), 1, "one dispatch");
    // Its values named by what their places do in the grammar, as the lean parser's (`rust_stream::meaning`).
    let variants: Vec<&str> = src.split("enum L {\n").nth(1).and_then(|e| e.split('}').next()).unwrap_or_default().lines().map(|l| l.trim().trim_end_matches(',')).filter(|l| !l.is_empty()).collect();
    assert_eq!(variants, ["ObjectBegin", "ArrayBegin", "ArrayValue", "End"], "the dispatch's values by their meaning");
    let resets = dispatch_resets(&src);
    assert!(resets.is_empty(), "a dispatch's arm only re-sets d: {resets:?}");
    // No room check a token: the harness sizes the tape from the input once, as simdjson's.
    assert!(!aot.parser.tier2.room_checked() && !src.contains("room()?"), "no room check a token");
    let entry = aot_entry(&source);
    // SAFETY: the source is this program's tier 2 (from its own native IR), reading and writing
    // `Ctx` as the JIT's does; it counts its calls open itself.
    unsafe { aot.parser.tier2.set_entry(entry, 4) };
    let inputs = tree_inputs();
    let (mut parsed, mut refused) = (0, 0);
    for (k, f) in inputs.iter().enumerate() {
        let show = || String::from_utf8_lossy(&f[..f.len().min(80)]).into_owned();
        let want = ours(&jit, f);
        assert_eq!(ours(&aot, f), want, "file {k} ({:?}): the Rust's nodes", show());
        // The tape's bytes alike too: each code, its supertypes' classes, the columns.
        if want.is_some() {
            assert!(aot.parser.image(f).ok() == jit.parser.image(f).ok(), "file {k} ({:?}): the Rust's tape image", show());
        }
        if want.is_some() { parsed += 1 } else { refused += 1 }
    }
    eprintln!("{} files: {parsed} parsed, {refused} refused, by the Rust and the JIT alike, tape bytes and all", inputs.len());
    // Format 16 (decision 0434): the Rust passes each node to the writer's ops at its word, as it is
    // made (`rust_source_16`), and writes the JIT's format 16 run's bytes, 16 and 16z: the ops' calls
    // are `native_cpu_tape::pass`'s, with no rows passed again.
    let (widths16, tag16) = jit.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    // The JIT's run in the simdjson form passes each node to the ops at its word (`direct`), as the Rust: no working
    // row, so no step on the rows is called.
    let s16 = jit.parser.sixteen().expect("a format 16 tier 2");
    assert!(s16.direct && s16.row_calls == 0, "the JIT's format 16 run: direct {}, {} calls of a step on the rows", s16.direct, s16.row_calls);
    assert_eq!(s16.tape(1000).held(), [0, 0], "the direct run's tape: no format 15 buffer, no working row");
    let lang = language_of(&jit, &inputs.iter().map(Vec::as_slice).collect::<Vec<_>>());
    let src16 = rerail_rust_stream::rust_stream::rust_source_16(&native, widths16, tag16, &lang).expect("tier 2 writing format 16, as Rust");
    if let Ok(dir) = std::env::var("RERAIL_DUMP_TIER2") {
        std::fs::write(std::path::Path::new(&dir).join("simdjson16.rs"), &src16).expect("writes the source");
    }
    let made16 = src16.matches("s.open16(").count() + src16.matches("s.leaf16(").count();
    // No working row: the rule touches neither rows nor supertype records, only the stack of open
    // nodes (a call's number in each) and the ops.
    let rule16 = src16.split("\nfn r0(").nth(1).expect("the rule");
    // Each node's number a literal at its word, as the lean parser's kind constants (decision 0535): no table read.
    assert!(!src16.contains("fn number16(") && src16.contains("pub const LANG: u64"), "a direct run's numbers are the language's literals");
    let reads: Vec<&str> = rule16.split("\n}\n").next().unwrap_or_default().lines().filter(|l| l.contains("numbers")).collect();
    assert!(reads.is_empty(), "a direct run's rule reads the numbers: {reads:?}");
    assert!(!rule16.contains("s.row(") && !rule16.contains("s.open(") && !rule16.contains("s.leaf::<") && !rule16.contains("s.node_super("), "no row in a format 16 run");
    // An end made only where a word reads it (`rust_stream::end_read`), as the lean parser makes a node's end where
    // it writes it: no `,`, `:`, `{` or `[` makes its end; `}` and `]` do, for the close of their container.
    let flat16 = flat(&src16);
    two_ways("16", &flat16);
    assert!(!src16.contains("part_at") && src16.contains("fn string(&mut self, mut at: u32, t: &mut T) -> R {"), "a direct run's string is given the opener's place");
    assert_eq!(src16.matches("match d {").count(), 1, "a direct run's one dispatch");
    for (byte, made) in [("0x2c", false), ("0x3a", false), ("0x7b", false), ("0x5b", false), ("0x7d", true), ("0x5d", true)] {
        let arms: Vec<&str> = flat16.split(&format!("\n{byte} => {{\n")).skip(1).collect();
        assert!(!arms.is_empty(), "the arms of {byte}");
        for arm in arms {
            // The last token's end kept first where a close after reads it (`rust_stream::last_ends`).
            let own = arm.strip_prefix("t.last_end = t.rec_end;\n").unwrap_or(arm);
            assert_eq!(own.starts_with("t.rec_end = t.start + 1;"), made, "{byte}'s arm makes its end only where it is read: {}", &arm[..arm.len().min(80)]);
            assert!(made || !arm.starts_with("t.last_end"), "{byte}'s arm keeps no end it does not make again");
        }
    }
    // A token taken as the lean parser's `take!()`: one load and one add, the stream ended in two entries of the
    // file's length (`Batch::end_streams`), and no copy of the last token's end (`rust_stream::last_ends`): a close
    // of it reads `rec_end` where no arm made it again, and `last_end` where the arm before it kept it.
    let take = src16.split("fn token_all(&mut self, t: &mut T) -> R {").nth(1).and_then(|r| r.split("\n    }\n").next()).expect("the token's step");
    assert_eq!(take.trim(), "t.start = unsafe { *self.starts.add(t.next as usize * self.rs) };\n        t.next += 1;\n        Ok(())", "the token's step");
    assert!(flat16.contains("\n0x2c => {\ns.close::<false, true>(t.rec_end);\n"), "a `,` closes the value by the end it left");
    assert!(flat16.contains("\n0x7d => {\nt.last_end = t.rec_end;\nt.rec_end = t.start + 1;\ns.close::<false, true>(t.last_end);\n"), "a `}}` keeps the value's end before it makes its own");
    // A close cannot fail (`rust_stream::sure_closes`, decision 0557): no result, no `?`, the writer's close asserts.
    assert!(src16.contains("    fn close<const THIS: bool, const SURE: bool>(&mut self, end: u32) {\n") && !src16.lines().any(|l| l.contains("s.close::<") && l.contains(")?;")), "a close returns nothing");
    assert!(src16.contains("debug_assert!(self.open_len != 0, \"a close of an open node\");") && !src16.contains("self.fail(w::fail::NO_OPEN"), "the writer's close cannot fail");
    // The language holds every key a direct run writes (`direct_entries`): no node word refuses for want of its
    // number (`rust_stream::UNNUMBERED`).
    let unnumbered = src16.matches(rerail_rust_stream::rust_stream::UNNUMBERED).count();
    assert_eq!(unnumbered, 0, "node words with no number in the language");
    // An empty object or array one leaf (`rust_stream::empties`, decision 0545), as the lean parser's `empty!`: each
    // opener's arm looks at the next token first, and where it is the closer writes a leaf, takes the closer and goes
    // on past the container; else it opens the container. Each such arm writes one more node call than format 15's.
    let empty = |closer: &str| format!("\nif s.byte(unsafe {{ *s.starts.add(t.next as usize * s.rs) }}) == {closer} {{\nif s.depth == DEPTH {{ return refuse(why::DEPTH); }}\nlet st = t.start;\ns.token_all(t)?;\nt.rec_end = t.start + 1;\ns.leaf16(");
    for (opener, closer) in [("0x5b", "0x5d"), ("0x7b", "0x7d")] {
        let arms: Vec<&str> = flat16.split(&format!("\n{opener} => {{")).skip(1).collect();
        assert!(!arms.is_empty(), "the arms of {opener}");
        for arm in arms {
            // Its enclosing nodes opened first (the document's, at the top, with its closer checked: 0559).
            let at = arm.find(&empty(closer));
            let first = at.is_some_and(|k| arm[..k].lines().all(|l| l.is_empty() || l.starts_with("s.open16(") || l.starts_with("if s.n == 0 || s.byte(unsafe { *s.starts.add((s.n - 1) * s.rs) })")));
            assert!(first, "{opener}'s arm looks for its closer first: {}", &arm[..arm.len().min(160)]);
        }
    }
    // The end tested once, after the root's value, by the stream's count (`rust_stream::counted_end`, decision 0546),
    // as the lean parser's `next == index + count`: no token taken for it, no other word tests it, `ACCEPT` neither.
    assert!(!src16.contains("t.start == s.len") && !src16.contains("t.start == self.len") && !src16.contains("t.at_end"), "the end tested once, by the count");
    assert!(flat16.contains("\nif (t.next as usize == s.n) {\n") && src16.contains("    fn accept(&mut self, t: &T) -> R {\n        if self.wr.open_len != 1 {"), "the end the stream's count, `ACCEPT` past it");
    // The document's container closed by the stream's last token, checked before the walk as the lean parser's
    // `last != '}'` (`rust_stream::last_closers`, decision 0559): in the `{` and `[` arms, once each.
    for closer in ["0x7d", "0x5d"] {
        let check = format!("if s.n == 0 || s.byte(unsafe {{ *s.starts.add((s.n - 1) * s.rs) }}) != {closer} {{ return refuse(why::BYTE); }}\n");
        assert_eq!(flat16.matches(&check).count(), 1, "the document's closer {closer} the last token, checked before the walk");
    }
    let empties = flat16.matches("\nlet st = t.start;\ns.token_all(t)?;\nt.rec_end = t.start + 1;\ns.leaf16(").count();
    assert!(empties > 0 && made16 == made + empties && src16.contains("mod ops {") && !src16.contains("self.finish(") && !src16.contains("s.code_now("), "each node to the ops as it is made, no image");
    // Written once: no format 15 cell (an offset, a block's base, a size, a tail) beside the ops' cells.
    // (The edits' methods, which no word of this program calls, keep theirs.)
    let body = |name: &str| src16.split(name).nth(1).and_then(|rest| rest.split("\n    }\n").next()).unwrap_or_else(|| panic!("{name}")).to_string();
    for name in ["    fn open16(", "    fn leaf16(", "    fn close<", "    fn accept(", "    fn keep_ret(", "    fn taken_ret("] {
        let b = body(name);
        assert!(!b.contains("self.push_off(") && !b.contains("self.close_cells(") && !b.contains("_c.add(") && !b.contains("self.row("), "no format 15 cells or rows in a format 16 run's {name}");
    }
    // The writer's room reserved once (`rust_stream::reserved`, decision 0544), as the lean parser's columns: a node's
    // `OPEN` and `LEAF` cannot fail, and no node's call is followed by `?`; no deferred extras (`pending`) in a direct
    // run; the nodes' count checked once, where the run starts.
    assert!(src16.contains("    fn open16(&mut self, number: u32, start: u32) {\n") && src16.contains("    fn leaf16(&mut self, number: u32, start: u32, end: u32) {\n"), "a node's ops return nothing");
    assert!(!src16.contains("unsafe fn open_node(&mut self, number: u32, start: u64, push: bool) -> Option<u32>") && src16.contains("unsafe fn open(&mut self, number: u32, start: u64) {\n"), "the writer's node ops cannot fail");
    let fails: Vec<&str> = src16.lines().filter(|l| (l.contains("s.open16(") || l.contains("s.leaf16(")) && l.trim_end().ends_with(")?;")).collect();
    assert!(fails.is_empty(), "a node's call that can refuse: {fails:?}");
    for name in ["    fn open16(", "    fn leaf16(", "    fn close<"] {
        assert!(!body(name).contains("self.pending"), "no deferred extras in a direct run's {name}");
    }
    assert!(src16.contains("if !unsafe { s.wr.fits(c.room_nodes) } {"), "the nodes' count checked once");
    let entry16 = aot_entry(&src16);
    // SAFETY: as the format 15 entry's, on the format 16 run's harness (`write16`: the writer and numbers set).
    unsafe { aot.parser.sixteen_mut().expect("a format 16 tier 2").set_entry(entry16, 4) };
    // The tape's home: the tape, then its presence (the document's and each block's), which the
    // ops make as they write.
    let bytes_of = |w: &rerail_tape_format::f16::Written| {
        assert!(w.presence_bytes().is_some(), "a format 16 run's presence");
        w.extent().to_vec()
    };
    let mut written = 0;
    for (k, f) in inputs.iter().enumerate() {
        let Ok(image) = jit.parser.image(f) else { continue };
        let (_, table) = rerail_tape_format::tape::f15_columns(&image).expect("a format 15 tape");
        let numbers = jit.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
        // The reference: format 15's tape written in format 16 by the writer (`f16::write_from_with_presence`), apart
        // from either run's passes; both runs pass each node at its word (`rust_stream::direct_keys`).
        let (columns, _) = rerail_tape_format::tape::f15_columns(&image).expect("a format 15 tape");
        let t = rerail_tape_format::tape::F15::new(&image, columns, &table).expect("reads");
        let number = rerail_tape_format::f16::numbers(&lang, &table).expect("numbers");
        for z in [false, true] {
            let reference = rerail_tape_format::f16::write_from_with_presence(&lang, &t, &number, f, tag16, z).expect("the writer's tape").tape;
            let want = jit.parser.image16(f, &lang, &numbers, z).map(|w| bytes_of(&w));
            let got = aot.parser.image16(f, &lang, &numbers, z).map(|w| bytes_of(&w));
            assert!(want.is_ok(), "file {k}: the JIT's format 16 run: {want:?}");
            assert!(want.as_ref().ok() == Some(&reference.extent().to_vec()), "file {k} (16z {z}): the JIT's format 16 home is not the writer's of its format 15 tape");
            if got != want {
                let (g, w) = (got.clone().unwrap_or_default(), want.clone().unwrap_or_default());
                let first = (0..g.len().min(w.len())).find(|&i| g[i] != w[i]);
                panic!("file {k} (16z {z}): the Rust's format 16 tape: {:?}, lengths {} and {}, first differing at {first:?}\n  rust {:?}\n  jit  {:?}", got.as_ref().err(), g.len(), w.len(), &g[..g.len().min(160)], &w[..w.len().min(160)]);
            }
        }
        written += 1;
    }
    assert_eq!(written, parsed, "every parsed file written in format 16");
    eprintln!("{written} files in format 16 and 16z, the Rust's bytes the JIT's");
    // Each refusal names its reason (`native_cpu_stream::refusal`, decision 0537): a file the JIT refuses, the Rust
    // refuses for the reason its word has.
    let numbers = jit.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
    let deep = deep_nests(1025..1026);
    let cases: [(&[u8], &str); 10] = [
        (b"[1 2]", "BYTE"),
        (b"{\"a\" 1}", "BYTE"),
        (b"[tru]", "BYTE"),
        (b"[01]", "CHECK"),
        (b"[\"\\q\"]", "BYTE"),
        // The region left open: tier 1's total of open regions, tested by the rule's first word (`EMPTY`).
        (b"\"open", "EMPTY"),
        // An escape at the file's end: the region is open too, and `EMPTY` refuses it first (`STRING` is not reached).
        (b"[\"\\", "EMPTY"),
        (&deep[0], "DEPTH"),
        (&deep[1], "DEPTH"),
        (&deep[2], "DEPTH"),
    ];
    for (f, why) in cases {
        let text = rerail_cpu_stream::native_cpu_stream::refusal::NAMES.iter().find(|n| n.0 == why).expect("a reason").1;
        assert!(jit.parser.image(f).is_err(), "{:?}: the JIT refuses it", String::from_utf8_lossy(&f[..f.len().min(20)]));
        let got = aot.parser.image16(f, &lang, &numbers, false).map(|_| ());
        assert!(got.as_ref().err().is_some_and(|e| e.contains(text)), "{:?}: refused for {why} ({text}): {got:?}", String::from_utf8_lossy(&f[..f.len().min(20)]));
    }
    // Empty containers, at the top, in arrays, as values in objects and at the depth's edge: the Rust's tapes the JIT's.
    for f in [&b"[]"[..], b"{}", b"[[]]", b"[{}]", b"{\"a\":[]}", b"{\"a\":{}}", b"[[],{},[[]],{\"b\":[]}]", b"{\"a\":[],\"b\":{},\"c\":1}", b"[ ]", b"{ }"] {
        for z in [false, true] {
            let want = jit.parser.image16(f, &lang, &numbers, z).map(|w| bytes_of(&w));
            let got = aot.parser.image16(f, &lang, &numbers, z).map(|w| bytes_of(&w));
            assert!(want.is_ok() && got == want, "{:?} (16z {z}): the Rust's tape is the JIT's", String::from_utf8_lossy(f));
        }
    }
    // A file cut off in a container, or with more after its value (the end the stream's count, decision 0546): its
    // tokens taken past the stream's count read the two entries there
    // (`Batch::end_streams`), the file's length, as the unclamped `TOKEN` takes them (decision 0543): the Rust refuses
    // each, as the JIT does (whose refusal names its word, not the reason). Ends of 0, as before, would read the
    // file's first byte as the next token's.
    for f in [&b"["[..], b"{", b"[1", b"[1,", b"[\"a\"", b"[\"a\",", b"{\"a\"", b"{\"a\":", b"{\"a\":1", b"{\"a\":1,", b"[[", b"[[1,", b"{\"a\":[", b"{\"a\":{\"b\":", b"", b"[1] 2", b"{} {}", b"\"a\" \"b\"", b"1 2", b"[]]", b"{}}", b"[1}", b"{\"a\":1]", b"[{}", b"{\"a\":[]", b"[1] ]", b"{} }", b"[1,2]}"] {
        let want = jit.parser.image16(f, &lang, &numbers, false).map(|_| ());
        let got = aot.parser.image16(f, &lang, &numbers, false).map(|_| ());
        assert!(want.is_err() && got.is_err(), "{:?}: the Rust refuses it as the JIT does: {got:?}, {want:?}", String::from_utf8_lossy(f));
    }
}

/// Page 40, order item 4: the simdjson form end to end against simdjson, on simdjson's own inputs (`RERAIL_JSON_FILES`,
/// `:`-separated; `twitter.json` and `numbers.json`). Ours: tier 1 on Cranelift, tier 2 the generated Rust (rustc,
/// `-C opt-level=3`) writing format 16 (`image16`), the tape's home and presence. simdjson's: its DOM parser, stage 1
/// and stage 2 to its tape, from a padded copy of the bytes (`structurals`). Per file: the instructions retired a byte
/// and the time a byte (each the median of `RERAIL_ITERS` runs), single-threaded.
#[cfg(target_arch = "aarch64")]
#[test]
#[ignore = "a measurement: run by name with RERAIL_JSON_FILES and --nocapture"]
fn simdjson_form_against_simdjson() {
    let files: Vec<(String, Vec<u8>)> = std::env::var("RERAIL_JSON_FILES")
        .expect("RERAIL_JSON_FILES")
        .split(':')
        .map(|p| (base(Path::new(p)), std::fs::read(p).expect("reads")))
        .collect();
    let iters: usize = std::env::var("RERAIL_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let (jit, mut aot) = (simdjson_form(), simdjson_form());
    let native = rerail_compiler::native_ir::lower(&aot.program, 0, 0).expect("lowers");
    let (widths16, tag16) = jit.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    let lang = language_of(&jit, &files.iter().map(|(_, f)| f.as_slice()).collect::<Vec<_>>());
    let src16 = rerail_rust_stream::rust_stream::rust_source_16(&native, widths16, tag16, &lang).expect("tier 2 writing format 16, as Rust");
    // SAFETY: the Rust of this program's tier 2 in format 16, on the format 16 run's harness.
    unsafe { aot.parser.sixteen_mut().expect("a format 16 tier 2").set_entry(aot_entry(&src16), 4) };
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
        jit.parser.image(f).unwrap_or_else(|e| panic!("{name}: the simdjson form refuses it: {e:?}"));
        let numbers = aot.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
        let want = jit.parser.image16(f, &lang, &numbers, false).map(|w| w.extent().to_vec()).expect("the JIT's format 16 run");
        assert_eq!(aot.parser.image16(f, &lang, &numbers, false).map(|w| w.extent().to_vec()).ok(), Some(want), "{name}: the Rust's tape is the JIT's");
        // Ours as simdjson's caller runs simdjson: one writer, kept from one document to the next (simdjson's parser is
        // kept by `structurals`), each tape given back as the next one's room.
        let mut w = rerail_tape_format::f16::Writer::empty();
        let mut ours = || {
            let t = aot.parser.image16_in(&mut w, f, &lang, &numbers, false).expect("parses");
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
        // And with a writer made for each document, as `image16` makes one.
        let (fi, _) = measure(&mut || {
            std::hint::black_box(aot.parser.image16(f, &lang, &numbers, false).expect("parses"));
        });
        let (si, sn) = measure(&mut || {
            std::hint::black_box(rerail_simdjson_native::structurals(f, |i| i.len()).expect("simdjson parses it"));
        });
        // Of ours, tier 1 alone, on a batch made once (its padded copy of the bytes outside the count), and with the
        // batch made each run.
        let mut batch = native_cpu_block::Batch::new(&aot.parser.tier1, &[f]);
        let (t1i, t1n) = measure(&mut || batch.call(&aot.parser.tier1));
        let (bi, bn) = measure(&mut || {
            let mut b = native_cpu_block::Batch::new(&aot.parser.tier1, &[f]);
            b.call(&aot.parser.tier1);
            std::hint::black_box(&b);
        });
        let b = f.len() as f64;
        println!("{name}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}", f.len(), oi / b, si / b, b / on, b / sn, oi / si, on / sn);
        println!("  of ours, ins a byte: a writer made for each document {:.2}", fi / b);
        println!("  of ours, ins a byte: tier 1 {:.2} ({:.2} ns), the batch made and tier 1 {:.2} ({:.2} ns), the rest {:.2} ({:.2} ns)", t1i / b, t1n / b, bi / b, bn / b, (oi - bi) / b, (on - bn) / b);
    }
}

/// `src`, tier 2's Rust, built by rustc (optimized, as the ahead-of-time build) into a library
/// and loaded: its entry.
#[cfg(target_arch = "aarch64")]
fn aot_entry(src: &str) -> rerail_cpu_stream::native_cpu_stream::Entry {
    // A directory a source: the loader gives back the library already loaded at a path, so two
    // sources built at one path would both run the first.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(src, &mut h);
    let dir = std::env::temp_dir().join(format!("rerail-json-aot-{}-{:x}", std::process::id(), std::hash::Hasher::finish(&h)));
    std::fs::create_dir_all(&dir).expect("a directory");
    let (rs, lib) = (dir.join("tier2.rs"), dir.join("libtier2.dylib"));
    std::fs::write(&rs, format!("{src}\n#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn rerail_tier2_entry(ctx: u64) -> u64 {{\n    unsafe {{ tier2_entry(ctx) }}\n}}\n")).expect("writes");
    let keep = std::env::var_os("RERAIL_KEEP_AOT").is_some();
    let out = std::process::Command::new("rustc").args(["--edition", "2024", "--crate-type", "cdylib", "-C", "opt-level=3"]).args(if keep { &["-g"][..] } else { &[][..] }).arg("-o").arg(&lib).arg(&rs).output().expect("runs rustc");
    assert!(out.status.success(), "rustc: {}", String::from_utf8_lossy(&out.stderr));
    let path = std::ffi::CString::new(lib.to_str().expect("a path")).expect("a path");
    // SAFETY: a library rustc just built from the source above; kept loaded for the process.
    let f = unsafe {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_NOW);
        assert!(!h.is_null(), "dlopen");
        let f = libc::dlsym(h, c"rerail_tier2_entry".as_ptr());
        assert!(!f.is_null(), "dlsym");
        std::mem::transmute::<*mut libc::c_void, rerail_cpu_stream::native_cpu_stream::Entry>(f)
    };
    // Loaded: the files are no longer needed (`RERAIL_KEEP_AOT`: kept, for a profiler to name the functions).
    if !keep {
        let _ = std::fs::remove_dir_all(&dir);
    }
    f
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
/// each word's byte ranges in it). Into `RERAIL_DUMP_TIER2`; `RERAIL_FORM`.
#[test]
#[ignore = "a dump: run by name with RERAIL_DUMP_TIER2"]
fn json_tier2_by_word() {
    let name = std::env::var("RERAIL_FORM").unwrap_or_else(|_| "json-rfc".to_owned());
    // Each function's IR kept, for its Rust (`rerail_clif::rust`): the run's option (decision 0442).
    let _run = rerail_context::test_run();
    rerail_compiler::native_cpu_codegen::options().keep_ir.store(true, std::sync::atomic::Ordering::Relaxed);
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
    // Tier 2 as generated Rust, to read beside simdjson's stage 2: the structured backend's
    // (`rust_stream`), and the compiler's own IR as Rust (`rerail_clif::rust`).
    let rust = rerail_rust_stream::rust_stream::rust_source(&native).unwrap_or_else(|e| format!("// not generated: {e}\n"));
    std::fs::write(format!("{dir}/{name}.rs"), rust).expect("writes");
    let t2 = &c.parser.tier2;
    let ir = rerail_cpu_stream::clif_rust::translate(&t2.ir, &[(t2.entry_id, "tier2")], &[]).unwrap_or_else(|e| format!("// not translated: {e}\n"));
    std::fs::write(format!("{dir}/{name}.ir.rs"), ir).expect("writes");
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
        std::fs::write(out, rerail_rir::rir_asm::disassemble(&c.program)).expect("writes");
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
}

/// The generated parser's format 16 and 16z tapes are the JIT's, byte for byte, in a language whose table is the
/// one the generator made (`ENTRIES`); it refuses the files the JIT refuses.
#[test]
fn json_rusts_tapes_are_the_jits() {
    let jit = simdjson_form();
    let (_, tag) = jit.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), [1, 1, 1], 1).expect("a language");
    let numbers = jit.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
    let mut inputs = tree_inputs();
    inputs.extend(deep_nests(1023..1026));
    let (mut same, mut refused) = (0, 0);
    for (k, f) in inputs.iter().enumerate() {
        let show = || String::from_utf8_lossy(&f[..f.len().min(60)]).into_owned();
        for z in [false, true] {
            let want = jit.parser.image16(f, &lang, &numbers, z).map(|w| w.extent().to_vec());
            let mut w = rerail_tape_format::f16::Writer::new(&lang, f, tag, z, true, f.len() / 2 + 16);
            let got = rerail_json_aot::parse(f, &mut w).map_err(|b| format!("refused at byte {b}")).and_then(|()| w.finish().map(|d| d.tape.extent().to_vec()));
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
