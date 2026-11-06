//! Stream programs: lexers that read bytes and write token masks, in the branch-free shape of
//! simdjson's stage 1 (`docs/parser-generators/09-compile-and-lower.md`). A second kind of
//! transducer program beside the tape walker (`packed`), over the Metal set steps' layout.
//!
//! - `syntax`, `mask`, `reference`: the Bitlex parser, its word operations and its reference
//!   evaluator, copied from `tools/bitlex`; the evaluator is the oracle.
//! - `ops`: the operations, every one a kernel of fixed cost per 64-byte block.
//! - `kernels`: those kernels.
//! - `lower`: a parsed lexer to operations.
//! - `interp`: the interpreter, an operation at a time over every block of a batch.
//! - `output`: error records.

#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod mask;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod reference;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod syntax;
pub(in crate::code_rule_dsl) mod interp;
pub(in crate::code_rule_dsl) mod kernels;
pub(in crate::code_rule_dsl) mod lower;
pub(in crate::code_rule_dsl) mod ops;
mod output;

use rayon::prelude::*;
use std::collections::HashMap;

/// The code point tables a lexer's `ucode` classes name, from tree-sitter's `parser.c`
/// (`BITLEX_PARSER_C`, or tree-sitter-python 0.25.0's in the cargo registry).
pub(in crate::code_rule_dsl) fn tables(prog: &[syntax::Stmt]) -> Result<HashMap<String, Vec<(u32, u32)>>, String> {
    let mut out = HashMap::new();
    for s in prog {
        if let syntax::Stmt::Ucode { classes, .. } = s {
            for (_, c) in classes {
                if let syntax::UClass::Table(file, sym) = c {
                    let path = std::env::var("BITLEX_PARSER_C").unwrap_or_else(|_| {
                        format!("{}/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-python-0.25.0/src/{file}", std::env::var("HOME").unwrap_or_default())
                    });
                    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
                    out.insert(format!("{file}:{sym}"), reference::load_table(&text, sym)?);
                }
            }
        }
    }
    Ok(out)
}

/// What a file's token stage reads: the errors so far, then each token's start and continuation
/// masks, each `munch` mask, and `skip`.
#[derive(PartialEq, Debug)]
pub(in crate::code_rule_dsl) struct MaskStage {
    pub errors: Vec<String>,
    pub masks: Vec<mask::Mask>,
}

/// Runs `prog` over `files` as one batch.
pub(in crate::code_rule_dsl) fn run(prog: &ops::Program, files: &[&[u8]]) -> Result<Vec<MaskStage>, String> {
    let batch = interp::Batch::new(files);
    let mut r = interp::Run::new(prog, &batch);
    r.exec()?;
    let mut errors: Vec<Vec<String>> = vec![vec![]; files.len()];
    let mut ids: Vec<&usize> = r.records.keys().collect();
    ids.sort();
    for id in ids {
        for (f, text) in &r.records[id] {
            errors[*f].push(text.clone());
        }
    }
    let mut out = vec![];
    for (f, errors) in errors.into_iter().enumerate() {
        let (first, blocks, len) = batch.files[f];
        let get = |id: usize| mask::Mask { n: len, w: r.m(id)[first..first + blocks].to_vec() };
        let mut masks = vec![];
        for t in &prog.tokens {
            masks.push(get(t.start));
            masks.push(t.through.map(get).unwrap_or_else(|| mask::Mask::zeros(len)));
        }
        masks.extend(prog.munch.iter().map(|m| get(m.1)));
        masks.push(get(prog.skip));
        out.push(MaskStage { errors, masks });
    }
    Ok(out)
}

fn reference_stage(o: reference::Output) -> MaskStage {
    let n = o.skip.n;
    let mut masks = vec![];
    for (_, s, t) in o.token_masks {
        masks.push(s);
        masks.push(t.unwrap_or_else(|| mask::Mask::zeros(n)));
    }
    masks.extend(o.munch_masks);
    masks.push(o.skip);
    MaskStage { errors: o.program_errors, masks }
}

/// The lexer `lexer` over the files listed in `list`: the stream program's mask stage against the
/// reference evaluator's, file by file, in batches of about `batch` bytes; each side's CPU time.
pub(in crate::code_rule_dsl) fn check(lexer: &str, list: &str, batch: usize) -> Result<String, String> {
    let prog = syntax::parse(lexer)?;
    let tabs = tables(&prog)?;
    let program = lower::lower(&prog, &tabs)?;
    let paths: Vec<String> = std::fs::read_to_string(list).map_err(|e| format!("{list}: {e}"))?.lines().map(String::from).collect();
    let srcs: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap_or_default()).collect();
    let bytes: usize = srcs.iter().map(Vec::len).sum();
    let cpu = crate::code_rule_dsl::prof::process_cpu;
    let t0 = cpu();
    let want: Vec<Result<MaskStage, String>> = srcs.par_iter().map(|s| reference::Ev::run(&prog, s, &tabs).map(reference_stage)).collect();
    let cpu_ref = cpu() - t0;
    let mut batches: Vec<std::ops::Range<usize>> = vec![];
    let (mut from, mut size) = (0, 0);
    for (i, s) in srcs.iter().enumerate() {
        if size > 0 && size + s.len() > batch {
            batches.push(from..i);
            (from, size) = (i, 0);
        }
        size += s.len();
    }
    if from < srcs.len() {
        batches.push(from..srcs.len());
    }
    let t1 = cpu();
    let got: Vec<Result<Vec<MaskStage>, String>> =
        batches.par_iter().map(|r| run(&program, &srcs[r.clone()].iter().map(Vec::as_slice).collect::<Vec<_>>())).collect();
    let cpu_stream = cpu() - t1;
    let (mut same, mut differ, mut failed) = (0, 0, 0);
    let mut shown = String::new();
    for (r, g) in batches.iter().zip(&got) {
        let g = match g {
            Ok(g) => g,
            Err(e) => {
                failed += r.len();
                shown += &format!("batch from {}: {e}\n", paths[r.start]);
                continue;
            }
        };
        for (i, gs) in r.clone().zip(g) {
            match &want[i] {
                Ok(w) if w == gs => same += 1,
                w => {
                    differ += 1;
                    if differ <= 5 {
                        shown += &format!("{}:\n", paths[i]);
                        match w {
                            Err(e) => shown += &format!("  reference failed: {e}\n"),
                            Ok(w) => {
                                if w.errors != gs.errors {
                                    shown += &format!("  errors: reference {:?}\n          stream    {:?}\n", w.errors, gs.errors);
                                }
                                for (k, (a, b)) in w.masks.iter().zip(&gs.masks).enumerate() {
                                    if a != b {
                                        let at = (0..a.n).find(|&j| a.get(j) != b.get(j)).unwrap_or(0);
                                        shown += &format!("  mask {k} first differs at byte {at}: reference {}, stream {}\n", a.get(at), b.get(at));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(format!(
        "{} files, {:.1} MB; {same} equal, {differ} differ, {failed} in failed batches\n\
         program: {} operations, {} live\n\
         reference: {:.2} s CPU ({:.1} MB/s)\n\
         stream ({} batches of about {} KB): {:.2} s CPU ({:.1} MB/s)\n{shown}",
        paths.len(),
        bytes as f64 / 1e6,
        program.ops.len(),
        program.live_ops(),
        cpu_ref.as_secs_f64(),
        bytes as f64 / 1e6 / cpu_ref.as_secs_f64().max(1e-9),
        batches.len(),
        batch / 1024,
        cpu_stream.as_secs_f64(),
        bytes as f64 / 1e6 / cpu_stream.as_secs_f64().max(1e-9),
    ))
}

#[cfg(test)]
mod tests {
    /// The rule of `ops.rs`, checked in the source: the kernels and the interpreter do not
    /// iterate over bytes, set bits or positions (`output.rs` does, and only it).
    #[test]
    fn stream_kernels_do_not_walk_bytes_or_bits() {
        for (name, src) in [("kernels.rs", include_str!("stream/kernels.rs")), ("interp.rs", include_str!("stream/interp.rs"))] {
            for bad in ["ones_iter", "trailing_zeros", "from_fn", ".get(i)", "for i in", "position("] {
                assert!(!src.contains(bad), "{name} contains `{bad}`");
            }
            // The one `while`: the interpreter's loop over operations.
            let whiles = src.matches("while ").count();
            let allowed = src.matches("while pc < n {").count();
            assert_eq!(whiles, allowed, "{name}: a `while` other than the loop over operations");
        }
    }
}
