//! `token_local`'s facts on the `.rr` grammar: every claim checked on its corpus, and two
//! censuses run by hand, of tier 1's operations by their facts and of its values' sparsity
//! (docs/parser-generators/25-token-local-work.md §5, 26 §1).

use rerail_compiler::bitlex_masks::Named;
use rerail_compiler::token_local;

/// Every claim on the `.rr` grammar's tier 1 holds on its corpus (a sample of files, every
/// claim on each), with each token kind's first bytes as the anchors.
#[test]
fn rr_facts_hold_on_the_corpus() {
    let c = super::compiled().expect("compiles");
    let starts: Vec<&Named> = c.values.iter().filter(|v| v.name.starts_with("token ") && v.name.ends_with(" start")).collect();
    assert!(starts.len() >= 8, "{starts:?}");
    let anchors: Vec<u16> = starts.iter().map(|v| v.op).collect();
    let facts = token_local::facts(&c.block, &anchors);
    let corpus = super::tests::corpus();
    // Every 25th file, and the longest: a few hundred kilobytes.
    let mut sample: Vec<&[u8]> = corpus.iter().step_by(25).map(|f| f.as_bytes()).collect();
    sample.extend(corpus.iter().max_by_key(|f| f.len()).map(|f| f.as_bytes()));
    // The marks of one word are the token's record's word in tier 2: their masks are gone from
    // tier 1 (`bitlex_masks::tree_section`).
    for word in ["fix_w", "suppress_w", "options_w", "over_w"] {
        assert!(c.values.iter().all(|v| v.name != word), "`{word}` is the record's word, not a mask");
    }
    // The other keyword masks are checks of one identifier: anchored on its first byte.
    let id = starts.iter().position(|v| v.name == "token identifier start").expect("identifiers");
    for word in ["input_w", "output_w", "agg_word"] {
        let v = c.values.iter().find(|v| v.name == word).unwrap_or_else(|| panic!("no `{word}`"));
        let fact = &facts[usize::from(v.op)];
        let shifts: Vec<i32> = fact.anchors.iter().filter(|a| a.0 == id).map(|&(_, s)| s + i32::from(starts[id].delay) - i32::from(v.delay)).collect();
        assert_eq!(shifts, [0], "`{word}` is anchored on an identifier's first byte: {:?}", fact.anchors);
        assert!(fact.local.is_some(), "`{word}` is local");
    }
    let checked = token_local::check(&c.block, &anchors, &facts, &sample);
    eprintln!(
        "{} files, {} bytes: {} anchors and {} local values checked ({} bits); {} wrong",
        sample.len(),
        sample.iter().map(|f| f.len()).sum::<usize>(),
        checked.anchored,
        checked.local,
        checked.bits,
        checked.wrong.len()
    );
    assert!(checked.wrong.is_empty(), "{:#?}", &checked.wrong[..checked.wrong.len().min(10)]);
    assert!(checked.local > 100 && checked.anchored > 20, "too few claims: {checked:?}");
}

/// How much of the `.rr` grammar's tier 1 is token-level: each operation by its facts, and the
/// share in the cones of values that only marks and the fallback read. A census for the plan's
/// payoff (doc 25 §5); run by hand.
#[test]
#[ignore = "a census: run by hand"]
fn rr_census() {
    let c = super::compiled().expect("compiles");
    let p = &c.block;
    let starts: Vec<&Named> = c.values.iter().filter(|v| v.name.starts_with("token ") && v.name.ends_with(" start")).collect();
    let anchors: Vec<u16> = starts.iter().map(|v| v.op).collect();
    let facts = token_local::facts(p, &anchors);
    let members = token_local::members(p);
    let n = p.ops.len();
    let (mut carried, mut positional, mut anchored, mut free) = (0, 0, 0, 0);
    for x in 0..n {
        if token_local::carried(p, &members, x) {
            carried += 1;
        } else if token_local::positional(&p.ops[x]) {
            positional += 1;
        } else if !facts[x].anchors.is_empty() {
            anchored += 1;
        } else {
            free += 1;
        }
    }
    eprintln!("{n} operations: {carried} carried, {positional} byte tests and positions, {anchored} local and anchored, {free} local, not anchored");
    // Where the unanchored local operations come from, by statement.
    let mut stmts: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    for x in 0..n {
        let e = stmts.entry(c.origin.get(x).map_or("?", String::as_str)).or_default();
        if token_local::carried(p, &members, x) {
            e.1 += 1;
        } else if !token_local::positional(&p.ops[x]) && facts[x].anchors.is_empty() {
            e.0 += 1;
        }
    }
    let mut stmts: Vec<_> = stmts.into_iter().filter(|s| s.1 .0 + s.1 .1 > 0).collect();
    stmts.sort_by_key(|s| std::cmp::Reverse(s.1 .0));
    eprintln!("local, not anchored (and carried), by statement:");
    for (s, (l, k)) in stmts.iter().take(40) {
        eprintln!("  {l:>4} ({k:>2})  {s}");
    }
    let kinds: Vec<String> = (0..p.outputs.len()).map(|k| c.labels.iter().filter(|l| usize::from(l.1) == k).map(|l| l.0.clone()).collect::<Vec<_>>().join("+")).collect();
    eprintln!("outputs: {kinds:?}");
    // The anchored values read by no carried value, by the outputs they reach.
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for x in 0..n {
        let f = &facts[x];
        if f.local.is_some() && !f.anchors.is_empty() && !token_local::positional(&p.ops[x]) {
            let reach = if f.carried_readers.is_empty() { "no carried reader" } else { "a carried reader" };
            let outs: Vec<&str> = f.outputs.iter().map(|&k| kinds[k].as_str()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            *by.entry(format!("{reach}; outputs {outs:?}")).or_default() += 1;
        }
    }
    for (k, v) in &by {
        eprintln!("  {v:>4}  {k}");
    }
    // The named values that are anchored, and who reads them.
    for v in &c.values {
        let f = &facts[usize::from(v.op)];
        if f.local.is_some() && !f.anchors.is_empty() && !token_local::positional(&p.ops[usize::from(v.op)]) && !v.name.starts_with("token ") {
            let cone = f.local.as_ref().map_or(0, |l| l.cone.len());
            eprintln!("  anchored {:<14} cone {cone:>3}  carried readers {:>2}  outputs {:?}", v.name, f.carried_readers.len(), f.outputs.iter().map(|&k| kinds[k].as_str()).collect::<std::collections::BTreeSet<_>>());
        }
    }
}

/// How sparse `.rr`'s tier 1 values are: for each named value, the share of the corpus's
/// 64-byte blocks where it has a bit, and, for each sparse one, the operations that are zero
/// wherever it is (by `zero_with`). The measure of simdjson's skip-when-absent branches
/// (doc 26); run by hand.
#[test]
#[ignore = "a census: run by hand"]
fn rr_sparsity() {
    let c = super::compiled().expect("compiles");
    let p = &c.block;
    let corpus = super::tests::corpus();
    let files: Vec<&[u8]> = corpus.iter().map(|f| f.as_bytes()).collect();
    let (values, spans) = p.all_values(&files);
    let blocks: usize = spans.iter().map(|s| s.len()).sum();
    let n = p.ops.len();
    let busy: Vec<usize> = (0..n).map(|x| values[x].iter().filter(|&&w| w != 0).count()).collect();
    let mut named: Vec<(&str, usize, u16)> = c.values.iter().filter(|v| !v.name.starts_with("token ")).map(|v| (v.name.as_str(), busy[usize::from(v.op)], v.op)).collect();
    named.sort_by_key(|v| v.1);
    eprintln!("{blocks} blocks; each named value's share of blocks with a bit, sparsest first:");
    for (name, b, op) in &named {
        eprintln!("  {:>6.2}%  {name} (%{op}, {:?})", 100.0 * *b as f64 / blocks as f64, p.ops[usize::from(*op)]);
    }
}

/// Tier 1's outputs and totals on `.rr`, and its passes' machine code: the measurement of
/// doc 26 §1.5.
#[test]
#[ignore = "a measurement: run by hand"]
fn rr_totals() {
    let c = super::compiled().expect("compiles");
    let b = &c.block;
    let labels: Vec<String> = c.labels.iter().map(|(l, k)| format!("{l}@{k}")).collect();
    eprintln!("outputs {} delays {:?}\ntotals {:?} extracts {:?} masks {:?} delay {}\nlabels {labels:?}", b.outputs.len(), b.output_delays, b.total_outputs(), b.extract_outputs(), b.mask_outputs(), b.delay);
    // Tier 1's passes' machine code: instructions, memory loads and stores, stack ones.
    let compiled = rerail_compiler::native_cpu_block::compile(&c.native).expect("compiles");
    let code = compiled.loops_disassembly();
    let lines: Vec<&str> = code.lines().map(str::trim).filter(|l| !l.is_empty() && !l.ends_with(':')).collect();
    let memory = |l: &str| ["ldr", "str", "ldp", "stp", "ldur", "stur"].iter().any(|m| l.starts_with(m));
    let stack = lines.iter().filter(|l| memory(l) && l.contains("[sp")).count();
    let mem = lines.iter().filter(|l| memory(l)).count() - stack;
    let bits = rerail_compiler::clif_text::state_bits(b);
    eprintln!("tier 1: {} ops, {bits} state bits, {} instructions, {mem} memory, {stack} stack", b.ops.len(), lines.len());
}


/// The most tier 1 can save by not storing the sparse marks' masks (doc 26 §1.7): its machine
/// code with and without their outputs.
#[test]
#[ignore = "a measurement: run by hand"]
fn rr_sparse_marks_bound() {
    let c = super::compiled().expect("compiles");
    let corpus = super::tests::corpus();
    let files: Vec<&[u8]> = corpus.iter().map(|f| f.as_bytes()).collect();
    let mb = files.iter().map(|f| f.len()).sum::<usize>() as f64 / 1e6;
    let count = |p: &rerail_compiler::block::BlockProgram| {
        let native = rerail_compiler::block_native::BlockNative::lower(p);
        let compiled = rerail_compiler::native_cpu_block::compile(&native).expect("compiles");
        let ms = compiled.batch_cpu_seconds(&files, 9) * 1e3;
        eprintln!("  tier 1 alone: {ms:.2} ms CPU over {mb:.2} MB");
        #[cfg(target_os = "macos")]
        {
            use rerail_metal::block_program::serial::{Segments, run_serial_split};
            let batch: Vec<rerail_compiler::block::BatchFile<'_>> = files.iter().map(|&bytes| rerail_compiler::block::BatchFile { bytes, program: 0 }).collect();
            let gpu = (0..5).map(|_| run_serial_split(&[&native], &batch, Segments { blocks: 29, warm: 1 }).expect("runs").run.seconds).fold(f64::MAX, f64::min);
            eprintln!("  tier 1 on Metal: {:.2} ms GPU", gpu * 1e3);
        }
        let code = compiled.loops_disassembly();
        let lines: Vec<&str> = code.lines().map(str::trim).filter(|l| !l.is_empty() && !l.ends_with(':')).collect();
        let memory = |l: &str| ["ldr", "str", "ldp", "stp", "ldur", "stur"].iter().any(|m| l.starts_with(m));
        let stack = lines.iter().filter(|l| memory(l) && l.contains("[sp")).count();
        (lines.len(), lines.iter().filter(|l| memory(l)).count() - stack, stack)
    };
    let sparse = ["options_on", "over_on", "no_value", "one_name", "fix_on", "empty_parens", "suppress_on"];
    let ops: Vec<u16> = c.values.iter().filter(|v| sparse.iter().any(|s| v.name == format!("mark {s}"))).map(|v| v.op).collect();
    // The marks' masks are `starts & (m | prev(fill(..)))` of these: the outputs the masks name.
    let mut p = c.block.clone();
    let mark_outputs: Vec<u16> = p.masks[1..1 + c.values.iter().filter(|v| v.name.starts_with("mark ")).count()].to_vec();
    eprintln!("mark ops {ops:?}; mark masks {mark_outputs:?}");
    let before = count(&p);
    // Drop the masks of the sparse marks, in the marks' order (`bitlex_masks::tree_section`).
    let names: Vec<&str> = c.values.iter().filter(|v| v.name.starts_with("mark ")).map(|v| v.name.as_str()).collect();
    let drop: Vec<u16> = names.iter().zip(&mark_outputs).filter(|(n, _)| sparse.iter().any(|s| **n == format!("mark {s}"))).map(|(_, &o)| o).collect();
    let keep: Vec<usize> = (0..p.outputs.len()).filter(|&k| !drop.contains(&p.outputs[k])).collect();
    p.output_delays = keep.iter().map(|&k| p.output_delays[k]).collect();
    p.outputs = keep.iter().map(|&k| p.outputs[k]).collect();
    p.masks.retain(|m| !drop.contains(m));
    let after = count(&p);
    eprintln!("names {names:?}\ntier 1 (instructions, memory, stack): with the sparse marks' masks {before:?}, without {after:?}");
}
