//! Stream programs: lexers that read bytes and write token masks, in the branch-free shape of
//! simdjson's stage 1 (`docs/parser-generators/09-compile-and-lower.md`). A second kind of
//! transducer program beside the tape walker (`packed`): it shares the space of files and the
//! set program's operations, not the walker's instructions (09 §7).
//!
//! - `syntax`, `mask`, `reference`: the Bitlex parser, its word operations and its reference
//!   evaluator, copied from `tools/bitlex`; the evaluator is the oracle.
//! - `program`: the stream program and its lowering from a parsed lexer.
//! - `cpu`: the interpreter, an operation at a time over a batch of files.
//! - `finish`: tokens from token masks.

#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod mask;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod reference;
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod syntax;
pub(in crate::code_rule_dsl) mod program;
pub(in crate::code_rule_dsl) mod cpu;
mod finish;

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

/// The lexer `lexer` over the files listed in `list`, by the reference evaluator and by the
/// stream program in batches of about `batch` bytes: whether each file's tokens and errors agree,
/// and each side's CPU time.
pub(in crate::code_rule_dsl) fn check(lexer: &str, list: &str, batch: usize) -> Result<String, String> {
    let prog = syntax::parse(lexer)?;
    let tabs = tables(&prog)?;
    let cpu0 = crate::code_rule_dsl::prof::process_cpu();
    let program = program::lower(&prog, &tabs)?;
    let cpu_lower = crate::code_rule_dsl::prof::process_cpu() - cpu0;
    let paths: Vec<String> = std::fs::read_to_string(list).map_err(|e| format!("{list}: {e}"))?.lines().map(String::from).collect();
    let srcs: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap_or_default()).collect();
    let bytes: usize = srcs.iter().map(Vec::len).sum();
    let cpu1 = crate::code_rule_dsl::prof::process_cpu();
    let want: Vec<Result<cpu::FileOut, String>> =
        srcs.par_iter().map(|s| reference::Ev::run(&prog, s, &tabs).map(|o| (o.tokens, o.errors))).collect();
    let cpu_ref = crate::code_rule_dsl::prof::process_cpu() - cpu1;
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
    let cpu2 = crate::code_rule_dsl::prof::process_cpu();
    let got: Vec<Result<Vec<cpu::FileOut>, String>> = batches
        .par_iter()
        .map(|r| cpu::run(&program, &srcs[r.clone()].iter().map(Vec::as_slice).collect::<Vec<_>>()))
        .collect();
    let cpu_stream = crate::code_rule_dsl::prof::process_cpu() - cpu2;
    let (mut same, mut differ, mut failed) = (0, 0, 0);
    let mut shown = String::new();
    for (r, g) in batches.iter().zip(&got) {
        let g = match g {
            Ok(g) => g,
            Err(e) => {
                failed += r.len();
                shown += &format!("batch of {} files from {}: {e}\n", r.len(), paths[r.start]);
                continue;
            }
        };
        for (i, (gt, ge)) in r.clone().zip(g) {
            let key = |t: &reference::Token| (t.kind.clone(), t.start, t.end, t.soft);
            let ok = match &want[i] {
                Ok((wt, we)) => wt.iter().map(key).eq(gt.iter().map(key)) && we == ge,
                Err(_) => false,
            };
            if ok {
                same += 1;
                continue;
            }
            differ += 1;
            if differ <= 5 {
                shown += &format!("{}:\n", paths[i]);
                match &want[i] {
                    Err(e) => shown += &format!("  reference failed: {e}\n"),
                    Ok((wt, we)) => {
                        if we != ge {
                            shown += &format!("  errors: reference {we:?}\n          stream    {ge:?}\n");
                        }
                        if let Some(k) = (0..wt.len().max(gt.len())).find(|&k| wt.get(k).map(key) != gt.get(k).map(key)) {
                            shown += &format!("  token {k}: reference {:?}, stream {:?}\n", wt.get(k).map(key), gt.get(k).map(key));
                        }
                    }
                }
            }
        }
    }
    let mut times: Vec<(String, f64)> = cpu::OP_TIMES.lock().unwrap().iter().map(|(k, v)| (k.clone(), *v)).collect();
    times.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (k, v) in times.iter().take(12) {
        shown += &format!("  {k:<12} {v:8.2} s CPU\n");
    }
    Ok(format!(
        "{} files, {:.1} MB; {same} equal, {differ} differ, {failed} in failed batches\n\
         program: {} operations, {} live; lowered in {:.3} s CPU\n\
         reference: {:.2} s CPU ({:.1} MB/s)\n\
         stream ({} batches of about {} KB): {:.2} s CPU ({:.1} MB/s)\n{shown}",
        paths.len(),
        bytes as f64 / 1e6,
        program.ops.len(),
        program.live_ops(),
        cpu_lower.as_secs_f64(),
        cpu_ref.as_secs_f64(),
        bytes as f64 / 1e6 / cpu_ref.as_secs_f64().max(1e-9),
        batches.len(),
        batch / 1024,
        cpu_stream.as_secs_f64(),
        bytes as f64 / 1e6 / cpu_stream.as_secs_f64().max(1e-9),
    ))
}
