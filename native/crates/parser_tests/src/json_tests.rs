//! The ladder's two JSON forms (`bitlex_ladder/14-json.bitlex`, `15-jsonc.bitlex`) against
//! simdjson and tree-sitter-json (docs/parser-generators/21-json-against-simdjson.md).
//!
//! The ladder's own test checks them as it checks every step. These add what only JSON has:
//! outside oracles, on files from outside.
//!
//! - **JSONTestSuite** (`RERAIL_JSON_SUITE`, its `test_parsing` directory): every `y_` file
//!   (valid JSON) is accepted by both forms; every `n_` file (invalid) is refused by step 14;
//!   step 15 refuses each too, unless it is valid once comments are allowed. Each file's
//!   compiled nodes equal the reference's.
//! - **A corpus** (`RERAIL_JSON_CORPUS`, simdjson's benchmark files): both forms accept each,
//!   with the same nodes, equal to tree-sitter-json's where it finds no error.
//! - **simdjson** (`RERAIL_SIMDJSON`, `tools/json-compare/simdjson_check`): its verdict on every
//!   file above, against step 14's.
//!
//! And `json_bench`, CPU time per file of each tier and of tree-sitter-json.
//!
//! The files are not in the repository; each test runs by name with its variables set.

use crate::json::{base, files_in, form};
use rerail_compiler::ladder::{Compiled, IdNode, as_ids, compile_step, reference};
use rerail_compiler::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};



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
    let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).expect("tree-sitter-json");
    let tree = parser.parse(bytes, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    fn walk(n: tree_sitter::Node<'_>, field: Option<&str>, out: &mut Vec<Named>) -> usize {
        let at = out.len();
        out.push((n.kind().to_owned(), field.unwrap_or("").to_owned(), n.start_byte(), n.end_byte(), 0));
        let mut size = 1;
        let mut c = n.walk();
        if c.goto_first_child() {
            loop {
                size += walk(c.node(), c.field_name(), out);
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        out[at].4 = size;
        size
    }
    let mut out = Vec::new();
    walk(tree.root_node(), None, &mut out);
    Some(out)
}

/// Ids back to names, for comparing with tree-sitter.
fn named(c: &Compiled, nodes: &[IdNode]) -> Vec<Named> {
    let names: HashMap<u16, String> = c.ids.0.borrow().iter().map(|(n, &i)| (i, n.clone())).collect();
    nodes
        .iter()
        .map(|n| {
            let field = if n.1 == 0 { String::new() } else { names[&n.1].trim_start_matches("field ").to_owned() };
            (names[&n.0].clone(), field, n.3, n.4, n.5)
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
    let (json, jsonc) = (form("14-json"), form("15-jsonc"));
    let paths = files_in("RERAIL_JSON_SUITE");
    let simd = simdjson(&paths);
    let mut problems: Vec<String> = Vec::new();
    // By the suite's class: [files, step 14 accepts, step 15 accepts, simdjson accepts, tree-sitter accepts].
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
            'n' if a.is_some() => problems.push(format!("{name}: invalid JSON accepted by step 14")),
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
            differ_simd.push(format!("{name} (simdjson {s}, step 14 {})", a.is_some()));
        }
    }
    eprintln!("class  files  step14  step15  simdjson  tree-sitter-json   (files accepted)");
    for (c, r) in &table {
        eprintln!("{c}_     {:5}  {:6}  {:6}  {:8}  {:5}", r[0], r[1], r[2], r[3], r[4]);
    }
    eprintln!("step 15 accepts, of the n_ files: {jsonc_takes:?}");
    eprintln!("tree-sitter-json refuses valid or accepts invalid: {ts_wrong:?}");
    eprintln!("step 14 and simdjson differ on {}: {differ_simd:#?}", differ_simd.len());
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// simdjson's benchmark files: both forms accept each, with the same nodes, and the nodes are
/// tree-sitter-json's where it finds no error.
#[test]
#[ignore = "needs a corpus: run by name with RERAIL_JSON_CORPUS (and RERAIL_SIMDJSON)"]
fn json_corpus_accuracy() {
    let (json, jsonc) = (form("14-json"), form("15-jsonc"));
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
            "{name}: {} bytes; step 14 {}, step 15 {}, simdjson {s:?}, tree-sitter {}; nodes {}; equal to tree-sitter's: {same_ts:?}",
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
    rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER1.store(opt.split(',').any(|t| t == "1"), std::sync::atomic::Ordering::Relaxed);
    rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER2.store(opt.split(',').any(|t| t == "2"), std::sync::atomic::Ordering::Relaxed);
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
        Err(_) => vec![("14-json".to_owned(), form("14-json")), ("15-jsonc".to_owned(), form("15-jsonc"))],
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

/// Cranelift's optimizer on JSON (doc 28): step 14 compiled four ways (optimizer off, on tier
/// 1, on tier 2, on both) in one process, then timed in turns: each round runs every form over
/// every file, tier 1 then tier 2, and each form keeps its best round. In turns, so the
/// machine's load falls on every form alike.
#[test]
#[ignore = "a benchmark: run by name with RERAIL_JSON_CORPUS"]
fn json_optimizer_bench() {
    use std::sync::atomic::Ordering::Relaxed;
    let rounds: usize = std::env::var("RERAIL_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let name = std::env::var("RERAIL_FORM").unwrap_or_else(|_| "14-json".to_owned());
    let paths = files_in("RERAIL_JSON_CORPUS");
    let files: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).expect("reads")).collect();
    let bytes: usize = files.iter().map(Vec::len).sum();
    let forms: Vec<(&str, Compiled)> = [("off", false, false), ("tier 1", true, false), ("tier 2", false, true), ("both", true, true)]
        .into_iter()
        .map(|(label, one, two)| {
            rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER1.store(one, Relaxed);
            rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER2.store(two, Relaxed);
            (label, form(&name))
        })
        .collect();
    rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER1.store(false, Relaxed);
    rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER2.store(false, Relaxed);
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
/// quote, which step 15 sends to tree-sitter by design, are counted apart.
#[test]
#[ignore = "needs JSONTestSuite and a corpus: run by name with RERAIL_JSON_SUITE and RERAIL_JSON_CORPUS"]
fn jsonc_comments_equal_tree_sitter() {
    let jsonc = form("15-jsonc");
    let mut paths: Vec<PathBuf> = files_in("RERAIL_JSON_SUITE").into_iter().filter(|p| base(p).starts_with("y_")).collect();
    paths.extend(files_in("RERAIL_JSON_CORPUS").into_iter().filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 100_000)));
    const CLEAN: [(&str, bool); 8] = [("// c", true), ("//", true), ("// a, [1] é\r", true), ("/* c */", false), ("/**/", false), ("/***/", false), ("/* a * b / c */", false), ("/*\n{\n*/", false)];
    const QUOTED: [(&str, bool); 2] = [("// \"x", true), ("/* \" */", false)];
    let mut rng = rerail_compiler::ladder::Rng(0x5eed_c0de);
    let (mut equal, mut quoted_refused, mut ts_errors, mut problems) = (0usize, 0usize, 0usize, Vec::new());
    for path in &paths {
        let bytes = std::fs::read(path).expect("reads");
        let Some(tree) = tree_sitter_json(&bytes) else { continue };
        // Where a comment may go: the start, after each leaf outside a string, after each string.
        let mut parent: Vec<usize> = vec![usize::MAX; tree.len()];
        let mut open: Vec<usize> = Vec::new();
        for i in 0..tree.len() {
            while open.last().is_some_and(|&o| i >= o + tree[o].4) {
                open.pop();
            }
            if let Some(&o) = open.last() {
                parent[i] = o;
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

/// Each form's shape, printed: tier 1's operations by kind, its carried state, its generated
/// loop body by opcode, and tier 2's words by opcode, with the machine code's sizes.
#[test]
#[ignore = "a measurement: run by name with --nocapture"]
fn json_shapes() {
    for name in ["14-json", "15-jsonc"] {
        let c = form(name);
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
    for name in ["14-json", "15-jsonc"] {
        let c = form(name);
        let mut rng = rerail_compiler::ladder::Rng(0x9e37_79b9_7f4a_7c15);
        let mut files: Vec<Vec<u8>> = Vec::new();
        for _ in 0..300 {
            let mut f = Vec::new();
            let len = 1 + rng.below(300) as usize;
            while f.len() < len {
                // Mostly ASCII, so that some blocks are all ASCII and others not.
                let p = if rng.below(4) == 0 { *rng.pick(&pieces) } else { &pieces[..2][rng.below(2) as usize] };
                f.extend_from_slice(p);
            }
            files.push(f);
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
        for f in &files {
            assert_eq!(c.parser.tier1.extract(f), block.run_files(&[f]).remove(0), "{name}: {:?}", String::from_utf8_lossy(f));
        }
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        assert_eq!(c.parser.tier1.run_files(&refs), block.run_files(&refs), "{name}: the batch");
        assert!(c.parser.tier1.ascii_branch, "{name}: no ASCII test");
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
    for name in ["14-json", "15-jsonc"] {
        let c = form(name);
        // Each region behind its branch in both full bodies, none computed in place.
        // A region with a test before its gate's has two branches.
        let plan = native_cpu_block::State::of(&c.native.block_programs[0]).plan;
        assert!(!plan.regions.is_empty(), "{name}: no gated region");
        let tests: usize = plan.regions.iter().map(|r| 1 + usize::from(r.pre.is_some())).sum();
        assert_eq!(c.parser.tier1.gate_branches, tests * (1 + usize::from(c.parser.tier1.ascii_branch)), "{name}: a region without its branch");
        let block = &c.program.block_programs[0];
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
            assert_eq!(c.parser.tier1.extract(f), block.run_files(&[f]).remove(0), "{name}: {:?}", String::from_utf8_lossy(f));
        }
    }
}

