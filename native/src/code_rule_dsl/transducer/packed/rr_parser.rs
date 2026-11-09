//! The `.rr` parser: the Bitlex grammar of `.rr` (the last step of `bitlex_ladder/rr/`), compiled
//! once per process through the Bitlex compiler to tier 1 and tier 2 on Cranelift, and shared by
//! every thread.
//!
//! `tape` gives the engine's own tape (`logic::rr::tape`): the same kinds, fields, spans and
//! parents. The grammar's kinds and fields take the engine's ids by name, so no table maps them.
//!
//! A file the compiled parser refuses goes to the engine's parser (`logic::rr::tape`). On the
//! corpus that is a file the engine refuses too, or a pathological one the grammar leaves to it
//! on purpose: a run of four backticks or more, braces nested past 127.

use super::bitlex_compile;
use super::native_cpu_stream::{Columns, NO_PARENT};
use super::{native_cpu_stream, native_ir, rir};
use crate::code_rule_dsl::logic::rr;
use std::cell::RefCell;
use std::collections::HashMap;

/// The grammar: the ladder's last step.
const GRAMMAR: &str = include_str!("bitlex_ladder/rr/20-json-depth.bitlex");

/// Ids above the engine's, for the grammar's own names (supertypes, marks) that never reach a
/// tape.
const OWN_IDS: u16 = 0x4000;

/// The compiled parser.
pub(super) struct Compiled {
    pub(super) parser: native_cpu_stream::Parser,
    /// Tier 1's cost by grammar statement, for the profile.
    pub(super) costs: Vec<bitlex_compile::Cost>,
    /// Each tier 1 operation's statement (`bitlex_compile::Compiled::origin`), for the profile.
    pub(super) origin: Vec<String>,
    /// Tier 1's named values (`bitlex_compile::Compiled::values`), for the analyses (`token_local`).
    pub(super) values: Vec<bitlex_compile::Named>,
    /// Tier 1's block program, as assembled.
    pub(super) block: super::block::BlockProgram,
    /// Tier 1's program as lowered, for its other executors (Metal).
    pub(super) native: super::native_ir::BlockNative,
    /// Tier 1's block program as text, for the profile's operation mix.
    pub(super) rows: String,
    /// Tier 2's words and its state machine as text, for the profile.
    pub(super) words: String,
    pub(super) machine: String,
}

// SAFETY: nothing changes a compiled parser once it is made. Its JIT module holds finished code
// and is only kept alive; `parse` takes `&self`, reads the code and its tables, and writes only
// buffers it allocates for the call. So threads may share one.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

/// The one compiled parser of the process, made at its first use.
static PARSER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled parser, compiling it the first time.
pub(super) fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(compile).as_ref().map_err(String::as_str)
}

fn compile() -> Result<Compiled, String> {
    let stmts = ::bitlex::syntax::parse(GRAMMAR)?;
    let own: RefCell<HashMap<String, u16>> = RefCell::new(HashMap::new());
    let id = |names: &[&str], name: &str| -> u16 {
        if let Some(at) = names.iter().position(|n| *n == name).filter(|&at| at > 0) {
            return at as u16;
        }
        let mut own = own.borrow_mut();
        let next = OWN_IDS + own.len() as u16;
        *own.entry(name.to_owned()).or_insert(next)
    };
    let kind = |n: &str| Some(if n == "ERROR" { u16::MAX } else { id(rr::tape::KIND_NAMES, n) });
    let field = |n: &str| Some(id(rr::tape::FIELD_NAMES, n.trim_start_matches("field ")));
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let compiled = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols))?;
    let program = rir::assemble(&compiled.text())?;
    let native = native_ir::lower(&program).map_err(|e| format!("lowers: {e:?}"))?;
    let parser = native_cpu_stream::Parser::new(&native).map_err(|e| format!("Cranelift: {e:?}"))?;
    Ok(Compiled { parser, native: native.block_programs[0].clone(), costs: compiled.costs.clone(), origin: compiled.origin.clone(), values: compiled.values.clone(), block: program.block_programs()[0].clone(), rows: compiled.rows.clone(), words: compiled.words.clone(), machine: compiled.machine.clone() })
}

/// `src`'s node table, as tier 2 writes it: from the compiled grammar, or, for a file the grammar
/// refuses, from the engine's parser (`logic::rr::tape`) in the same columns.
pub(in crate::code_rule_dsl) fn tape(src: &str) -> Columns {
    compiled_tape(src).unwrap_or_else(|| engine_columns(&rr::tape(src)))
}

/// The compiled grammar's node table, or `None` where it refuses the file (or did not compile).
/// Every column is the compiled code's: kinds, fields, spans and sizes from tier 2's words,
/// parents and extra flags from its final pass.
pub(in crate::code_rule_dsl) fn compiled_tape(src: &str) -> Option<Columns> {
    compiled().ok()?.parser.columns(src.as_bytes()).ok()
}

/// The node tables of `files`, in order, as `tape` gives each: tier 1 over the whole batch on
/// the device (Metal's forward carry), then tier 2 over each file reading tier 1's buffers where
/// the device left them; without a device, `tape` file by file.
pub(in crate::code_rule_dsl) fn tapes(files: &[&str]) -> Vec<Columns> {
    let mut out = Vec::with_capacity(files.len());
    #[cfg(target_os = "macos")]
    if let Ok(c) = compiled() {
        if metal_tapes(c, files, &mut out) {
            return out;
        }
        out.clear();
    }
    for file in files {
        out.push(tape(file));
    }
    out
}

/// Tier 1's segments on the device: 29 blocks and a warm-up block each side, a SIMD group.
#[cfg(target_os = "macos")]
const SEGMENTS: super::metal_program_backend::block_program::serial::Segments = super::metal_program_backend::block_program::serial::Segments { blocks: 29, warm: 1 };

/// `tapes` on Metal: halves while the device refuses the batch for capacity; `false` when it
/// cannot run it at all.
#[cfg(target_os = "macos")]
fn metal_tapes(c: &Compiled, files: &[&str], out: &mut Vec<Columns>) -> bool {
    use super::metal_program_backend::block_program::serial::run_resident;
    let mut batch = Vec::with_capacity(files.len());
    for file in files {
        batch.push(super::block::BatchFile { bytes: file.as_bytes(), program: 0 });
    }
    match run_resident(&[&c.native], &batch, SEGMENTS) {
        Ok((_, run)) => {
            let mut i = 0;
            for file in files {
                out.push(c.parser.columns_on(&run, i).unwrap_or_else(|_| engine_columns(&rr::tape(file))));
                i += 1;
            }
            true
        }
        Err(super::Error::BackendCapacity) if files.len() > 1 => {
            let (a, b) = files.split_at(files.len() / 2);
            metal_tapes(c, a, out) && metal_tapes(c, b, out)
        }
        Err(_) => false,
    }
}

/// The fallback: the engine's parser's nodes, in the compiled parser's columns.
pub(in crate::code_rule_dsl) fn engine_columns(nodes: &[rr::tape::Node]) -> Columns {
    let mut c = Columns { size: vec![1; nodes.len()], ..Columns::default() };
    for n in nodes {
        c.kind.push(u32::from(n.kind));
        c.field.push(u32::from(n.field));
        c.start.push(n.start);
        c.end.push(n.end);
        c.parent.push(n.parent.unwrap_or(NO_PARENT));
        c.extra.push(u8::from(n.extra));
    }
    // In preorder a node comes after its parent: sizes from the last node back.
    for (i, n) in nodes.iter().enumerate().rev() {
        if let Some(p) = n.parent {
            c.size[p as usize] += c.size[i];
        }
    }
    c
}

/// Why the grammar did not compile, if it did not: a test's check.
#[cfg(test)]
pub(super) fn compile_error() -> Option<String> {
    compiled().err().map(str::to_owned)
}

#[cfg(test)]
pub(super) mod tests {
    /// The compiled grammar's tape is the engine's, node for node (extras and parents too), on
    /// documents it parses; the ones it leaves to the engine come back from there.
    #[test]
    fn tape_is_the_engines() {
        assert_eq!(super::compile_error(), None);
        let parsed = [
            "r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n",
            "r(n) :- n = sum min x : { s(x) } : { t(n) }, m = count : { u(_) }.",
            ".policy {\"a\": [1, {\"b\": null}]}\n/* d */ r(x) :- x ~ `a`, y = \"s\".",
        ];
        for src in parsed {
            assert!(super::compiled_tape(src).is_some(), "the grammar refuses {src:?}");
            assert_eq!(super::tape(src), super::engine_columns(&super::rr::tape(src)), "{src:?}");
        }
        let deep = format!(".policy {{\"a\": {}{}}}", "[".repeat(127), "]".repeat(127));
        for src in ["r(x) :- x ~ ````a````.", deep.as_str(), "r(x) :- ."] {
            assert!(super::compiled_tape(src).is_none(), "the grammar takes {src:?}");
            assert_eq!(super::tape(src), super::engine_columns(&super::rr::tape(src)), "{src:?}");
        }
    }

    /// The parse path is the compiled code's: the host functions around the two compiled calls
    /// (layout, buffers, the columns taken) hold no loop over bytes, tokens or nodes, and call
    /// no Rust that walks them. Each function on the path is read from its source and refused if
    /// its body has a loop or an iterator walk; the only loops allowed are over a batch's files
    /// and over tier 1's compiled passes. The engine's parser, for a file the grammar refuses, is
    /// the one way out of the path. The compiled code's own loops are checked where it is
    /// made (`native_cpu_shape`: each loop steps a block, a token or a record, never a byte).
    #[test]
    fn the_parse_path_is_compiled_code() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/transducer/packed");
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).expect("the source");
        // The body of the function whose signature starts with `head`.
        let body = |src: &str, head: &str| -> String {
            let at = src.find(head).unwrap_or_else(|| panic!("no `{head}`"));
            let open = at + src[at..].find('{').expect("a body");
            let mut depth = 0usize;
            for (k, ch) in src[open..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return src[open..open + k + 1].to_owned();
                        }
                    }
                    _ => {}
                }
            }
            panic!("`{head}`: no end")
        };
        let path = [
            ("rr_parser.rs", "pub(in crate::code_rule_dsl) fn tape(src: &str)"),
            ("rr_parser.rs", "pub(in crate::code_rule_dsl) fn compiled_tape("),
            ("native_cpu_stream.rs", "pub(super) fn columns(&self, bytes: &[u8])"),
            ("native_cpu_stream.rs", "pub(super) fn columns(mut self)"),
            ("native_cpu_stream.rs", "pub(super) fn tape(&self, batch"),
            ("native_cpu_stream.rs", "pub(super) fn tape(&self, n: usize)"),
            ("native_cpu_stream.rs", "pub(super) fn words(&self, batch"),
            ("native_cpu_stream.rs", "fn stream<'b>(&self"),
            ("native_cpu_stream.rs", "pub(super) unsafe fn call(&self, bytes"),
            ("native_cpu_stream.rs", "pub(super) fn columns_on(&self"),
            ("native_cpu_stream.rs", "unsafe fn call_once(&self"),
            ("native_cpu_stream.rs", "fn full_tape(&self"),
            ("native_cpu_stream.rs", "fn sized(&self"),
            ("rr_parser.rs", "pub(in crate::code_rule_dsl) fn tapes("),
            ("rr_parser.rs", "fn metal_tapes("),
            ("native_cpu_block.rs", "pub(super) fn new(program: &CompiledBlock, files"),
            ("native_cpu_block.rs", "pub(super) fn call(&mut self, program: &CompiledBlock)"),
        ];
        let walks = [".iter(", ".iter_mut(", ".into_iter(", ".chunks", ".windows(", ".flatten(", ".map(", ".fold(", ".for_each(", ".bytes()", ".chars()", ".collect", ".position(", ".find(", ".any(", ".all(", ".filter(", ".sum(", ".count()", "while ", "loop {", "nodes()"];
        let loops_allowed = ["for file in files", "for entry in &program.entries"];
        for (file, head) in path {
            // The code alone: comments dropped.
            let b: String = body(&read(file), head).lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
            for w in walks {
                assert!(!b.contains(w), "{file} `{head}`: `{w}` on the parse path");
            }
            for (k, _) in b.match_indices("for ") {
                assert!(loops_allowed.iter().any(|a| b[k..].starts_with(a)), "{file} `{head}`: a loop on the parse path: {}", &b[k..(k + 40).min(b.len())]);
            }
        }
    }

    /// Tier 1 of the grammar on Metal (the forward carry: its fixed points, its passes for the
    /// `back_fill`s, its position planes) equals Cranelift's over the corpus as one batch: every
    /// output word, every position, every total. Prints the GPU time.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "the corpus on Metal: run by hand"]
    fn tier1_on_metal_equals_cranelift() {
        use super::super::block;
        use super::super::metal_program_backend::block_program::serial::{Segments, run_serial_split};
        let c = super::compiled().expect("compiles");
        let files = corpus();
        let refs: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        let want = c.parser.tier1.run_files(&refs);
        let batch: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
        let native = &c.native;
        let got = run_serial_split(&[native], &batch, Segments { blocks: 29, warm: 1 }).expect("Metal runs tier 1");
        let mut differ = 0;
        for (k, (g, w)) in got.run.files.iter().zip(&want).enumerate() {
            if g.outputs != w.outputs || g.positions != w.positions || g.totals != w.totals {
                differ += 1;
                if differ < 4 {
                    let o = (0..w.outputs.len()).find(|&o| g.outputs.get(o) != w.outputs.get(o));
                    eprintln!("file {k} ({} bytes) differs: output {o:?}, positions equal {}, totals {:?} / {:?}", refs[k].len(), g.positions == w.positions, g.totals, w.totals);
                }
            }
        }
        let bytes: usize = refs.iter().map(|b| b.len()).sum();
        eprintln!("{} files, {bytes} bytes: {differ} differ; GPU {:.2} ms ({:.2} GB/s), {} launches, {} segments, {} reruns", files.len(), got.run.seconds * 1e3, bytes as f64 / got.run.seconds / 1e9, got.run.launches, got.segments, got.reruns);
        assert_eq!(differ, 0);
    }

    /// `tapes` (tier 1 on Metal, tier 2 over its buffers in place) gives `tape`'s tables: on
    /// small documents, the ones the grammar refuses among them.
    #[test]
    fn tapes_are_tape() {
        let deep = format!(".policy {{\"a\": {}{}}}", "[".repeat(127), "]".repeat(127));
        let long = "r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n".repeat(200);
        let files = ["r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n", "r(x) :- ````a````.", ".policy {\"a\": [1, {\"b\": null}]}\n/* d */ r(x) :- x ~ `a`, y = \"s\".", deep.as_str(), "", long.as_str()];
        let got = super::tapes(&files);
        #[cfg(target_os = "macos")]
        {
            let mut on = Vec::new();
            assert!(super::metal_tapes(super::compiled().expect("compiles"), &files, &mut on), "Metal runs the batch");
            assert_eq!(on, got);
        }
        for (src, g) in files.iter().zip(&got) {
            assert_eq!(*g, super::tape(src), "{src:?}");
        }
    }

    /// `tapes` over the corpus as one batch equals `tape` file by file. Prints the time.
    #[test]
    #[ignore = "the corpus on Metal: run by hand"]
    fn corpus_tapes_are_tape() {
        let files = corpus();
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let _ = super::compiled();
        let t = cpu();
        let got = super::tapes(&refs);
        eprintln!("tapes: {} files in {:?} CPU", refs.len(), cpu() - t);
        let differ = refs.iter().zip(&got).filter(|(src, g)| super::tape(src) != **g).count();
        assert_eq!(differ, 0);
    }

    /// Tier 2's words by operation, and its state machine's size: what makes the function big.
    #[test]
    #[ignore = "a profile: run by hand"]
    fn rr_words_mix() {
        let c = super::compiled().expect("compiles");
        let mut mix: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for l in c.words.lines() {
            let op = l.split_whitespace().find(|w| w.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_') && w.len() > 1).unwrap_or("?");
            *mix.entry(op.to_owned()).or_default() += 1;
        }
        let mut v: Vec<_> = mix.into_iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("words: {} lines; machine: {} lines", c.words.lines().count(), c.machine.lines().count());
        eprintln!("{v:?}");
        let at = c.words.lines().position(|l| l.contains("TOKEN")).unwrap_or(0) + 30000;
        eprintln!("lines:\n{}", c.words.lines().skip(at).take(250).collect::<Vec<_>>().join("\n"));
    }

    /// The process's CPU time so far, user and sys: the one thread runs alone.
    fn cpu() -> std::time::Duration {
        // SAFETY: getrusage writes the zeroed structure.
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let time = |t: libc::timeval| std::time::Duration::from_secs(t.tv_sec as u64) + std::time::Duration::from_micros(t.tv_usec as u64);
        time(usage.ru_utime) + time(usage.ru_stime)
    }

    /// CPU time over the repository's `.rr` files: the compiled grammar against the engine's
    /// parser, each over every file `REPEAT` times, the grammar compiled before. Run alone
    /// (`--test-threads 1`) and at nice 19; the machine's load is printed.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_parser_speed() {
        const REPEAT: usize = 5;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the repository").to_path_buf();
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "rr") {
                    files.extend(std::fs::read_to_string(&p).ok());
                }
            }
        }
        let t = cpu();
        assert_eq!(super::compile_error(), None);
        let compile = cpu() - t;
        let bytes: usize = files.iter().map(String::len).sum();
        let ours: usize = files.iter().filter(|f| super::compiled_tape(f).is_some()).count();
        let run = |f: &dyn Fn(&str) -> usize| {
            let t = cpu();
            let mut nodes = 0;
            for _ in 0..REPEAT {
                for src in &files {
                    nodes += f(src);
                }
            }
            std::hint::black_box(nodes);
            (cpu() - t) / REPEAT as u32
        };
        let compiled = run(&|s| super::compiled_tape(s).map_or(0, |t| t.len()));
        let engine = run(&|s| super::rr::tape(s).len());
        let mb = bytes as f64 / 1e6;
        let load = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
        eprintln!("{} files, {:.2} MB, {ours} parsed by the grammar; compile {compile:.2?} CPU", files.len(), mb);
        eprintln!("  grammar: {compiled:.3?} CPU a pass, {:.1} MB/s", mb / compiled.as_secs_f64());
        eprintln!("  engine:  {engine:.3?} CPU a pass, {:.1} MB/s", mb / engine.as_secs_f64());
        eprintln!("  load: {load}");
    }

    /// This thread's CPU time (user and sys), to the nanosecond.
    fn thread_cpu() -> std::time::Duration {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: clock_gettime writes the structure.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
        std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
    }

    /// The repository's `.rr` files.
    pub(in crate::code_rule_dsl::transducer::packed) fn corpus() -> Vec<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the repository").to_path_buf();
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "rr") {
                    files.extend(std::fs::read_to_string(&p).ok());
                }
            }
        }
        files.sort();
        files
    }

    /// Where the parser's time goes, and what in its code makes it slow. Over the corpus, CPU
    /// per stage of `Parser::parse` and of the tape's conversion (each stage summed over the
    /// files, a pass's mean of `REPEAT`); tier 1 over the files as one batch (its speed without
    /// the per-file cost); then hazards in the machine code: its size, its stack spills and
    /// reloads, its branches and fixed points' loops; and why tier 1 spills: each pass's
    /// register pressure in its hot loop before allocation, and the values most live while it
    /// is over the registers, each named by its row and Bitlex statement
    /// (`native_cpu_pressure`). `RERAIL_RR_PROF_DIR`: each function's
    /// Cranelift IR and machine code written there, for a closer look. Run alone, at nice 19.
    #[test]
    #[ignore = "a profile: run by hand"]
    fn rr_parser_profile() {
        use super::super::native_cpu_block::Batch;
        const REPEAT: u32 = 5;
        let c = super::compiled().expect("compiles");
        let p = &c.parser;
        let files = corpus();
        let bytes: usize = files.iter().map(String::len).sum();
        let mb = bytes as f64 / 1e6;
        let mut stage = [std::time::Duration::ZERO; 5];
        let names = ["layout (Batch::new)", "tier 1 (masks, extracts)", "tape allocated", "tier 2 (words)", "columns taken"];
        let (mut nodes_n, mut refused) = (0usize, 0usize);
        for _ in 0..REPEAT {
            for src in &files {
                let b = src.as_bytes();
                let mut t = thread_cpu();
                let mut lap = |k: usize, t: &mut std::time::Duration| {
                    let now = thread_cpu();
                    stage[k] += now - *t;
                    *t = now;
                };
                let mut batch = Batch::new(&p.tier1, &[b]);
                lap(0, &mut t);
                batch.call(&p.tier1);
                lap(1, &mut t);
                let mut tape = p.tape(&batch);
                lap(2, &mut t);
                let ok = p.words(&batch, &mut tape).is_ok();
                lap(3, &mut t);
                if !ok {
                    refused += 1;
                    continue;
                }
                let out = tape.columns();
                lap(4, &mut t);
                nodes_n += out.len();
            }
        }
        let total: std::time::Duration = stage.iter().sum();
        eprintln!("{} files, {mb:.2} MB, {} nodes, {} refused (a pass)", files.len(), nodes_n / REPEAT as usize, refused / REPEAT as usize);
        eprintln!("  {:<28} {:>10} {:>9} {:>6}", "stage", "CPU/pass", "MB/s", "share");
        for (k, name) in names.iter().enumerate() {
            let d = stage[k] / REPEAT;
            eprintln!("  {name:<28} {:>8.1}ms {:>9.1} {:>5.1}%", d.as_secs_f64() * 1e3, mb / d.as_secs_f64(), 100.0 * stage[k].as_secs_f64() / total.as_secs_f64());
        }
        let d = total / REPEAT;
        eprintln!("  {:<28} {:>8.1}ms {:>9.1}", "all", d.as_secs_f64() * 1e3, mb / d.as_secs_f64());
        // The tape's room against what is written: rows allocated (and zeroed) for every file.
        let (mut room, mut used) = (0usize, 0usize);
        for src in &files {
            let mut batch = Batch::new(&p.tier1, &[src.as_bytes()]);
            batch.call(&p.tier1);
            let mut tape = p.tape(&batch);
            room += tape.kind.len();
            if p.words(&batch, &mut tape).is_ok() {
                used += tape.counts[0] as usize;
            }
        }
        let retries = super::super::native_cpu_stream::ROOM_RETRIES.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!("tape rows: {room} allocated, {used} written ({:.1}x), {} MB zeroed a pass; {retries} runs again on the bound's tape (all passes)", room as f64 / used.max(1) as f64, room * 4 * 7 / 1_000_000);
        // Tier 1 alone, every file in one batch.
        let refs: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        let t = thread_cpu();
        let mut batch = Batch::new(&p.tier1, &refs);
        let layout = thread_cpu() - t;
        let t = thread_cpu();
        for _ in 0..REPEAT {
            batch.call(&p.tier1);
        }
        let one = (thread_cpu() - t) / REPEAT;
        let blocks: u64 = batch.rows.iter().map(|r| r.blocks).sum();
        eprintln!("tier 1 as one batch: {:.1}ms CPU ({:.1} MB/s, {:.1} ns a 64-byte block); its layout {:.1}ms", one.as_secs_f64() * 1e3, mb / one.as_secs_f64(), one.as_secs_f64() * 1e9 / blocks as f64, layout.as_secs_f64() * 1e3);
        eprintln!("  outputs stored a block: {} words ({} bytes a 64-byte block)", p.tier1.outputs, p.tier1.outputs * 8);
        // Where tier 1's operations come from: the grammar's statements, costliest first.
        let mut costs = c.costs.clone();
        costs.sort_by_key(|k| std::cmp::Reverse(k.ops));
        let ops: u32 = costs.iter().map(|k| k.ops).sum();
        let backs: u32 = costs.iter().map(|k| k.back_fills).sum();
        eprintln!("tier 1 block program: {ops} operations, {backs} back_fill's; costliest statements:");
        for k in costs.iter().take(25) {
            eprintln!("  {:>5} ops {:>3} back_fill  {}", k.ops, k.back_fills, k.what);
        }
        // The operation mix: each row's operation name.
        let mut mix: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for line in c.rows.lines() {
            // `block 0 %k = op …`
            if let Some((_, rhs)) = line.split_once(" = ")
                && let Some(op) = rhs.split_whitespace().next()
            {
                *mix.entry(op.to_owned()).or_default() += 1;
            }
        }
        let mut mix: Vec<_> = mix.into_iter().collect();
        mix.sort_by_key(|m| std::cmp::Reverse(m.1));
        eprintln!("  operation mix: {}", mix.iter().take(16).map(|(o, n)| format!("{o} {n}")).collect::<Vec<_>>().join(", "));
        // Hazards in the machine code, a pass at a time: spills (stack loads and stores) and
        // conditional branches.
        let asm = |text: &str| {
            let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.ends_with(':') && !l.starts_with(';')).collect();
            let has = |pat: &[&str]| lines.iter().filter(|l| pat.iter().any(|p| l.contains(p))).count();
            let branch = lines.iter().filter(|l| ["b.", "cbz", "cbnz", "tbz", "tbnz", "br_table"].iter().any(|p| l.starts_with(p))).count();
            (lines.len(), has(&["[sp", "[fp", "nominal_sp", "spillslot"]), branch)
        };
        let (n1, sp1, br1) = asm(&p.tier1.disassembly);
        let (n2, sp2, br2) = asm(&p.tier2.disassembly);
        eprintln!("tier 1 code: {n1} instructions, {sp1} stack loads/stores (spills), {br1} conditional branches; {} passes", p.tier1.passes.len());
        for (k, pass) in p.tier1.passes.iter().enumerate() {
            let (n, sp, br) = asm(&pass.disassembly);
            eprintln!("  pass {k}: {:<8} {:>5} CLIF lines {n:>5} instructions {sp:>5} spills {br:>4} branches; gated {}, loops {}, ASCII branch {}", if pass.back { "backward" } else { "forward" }, pass.clif.lines().count(), pass.gate_branches, pass.round_branches, pass.ascii_branch);
        }
        eprintln!("tier 2 code: {n2} instructions, {sp2} stack loads/stores, {br2} conditional branches");
        // Why tier 1 spills: what is live where, before register allocation
        // (`native_cpu_pressure`). Each value is named by the operation that made it (its row
        // in `tier1.rows`) and the Bitlex statement that made the operation.
        let mut row_text: std::collections::HashMap<u32, String> = Default::default();
        for line in c.rows.lines() {
            if let Some((lhs, rhs)) = line.split_once(" = ")
                && let Some(k) = lhs.split_whitespace().last().and_then(|v| v.strip_prefix('%')).and_then(|v| v.parse().ok())
            {
                row_text.insert(k, rhs.chars().take(28).collect());
            }
        }
        let name = |o: Option<u32>, made: &str| match o {
            None => format!("{made:<14} (unmarked: the loop's, not a value's)"),
            Some(o) => {
                let stmt = c.origin.get(o as usize).filter(|s| !s.is_empty()).map_or("(lowering)", String::as_str);
                format!("{made:<14} %{o:<5} {:<28} {stmt}", row_text.get(&o).map_or("?", String::as_str))
            }
        };
        use super::super::native_cpu_pressure::{INT_REGS, VEC_REGS};
        eprintln!("tier 1 register pressure (before allocation; ~{INT_REGS} integer, ~{VEC_REGS} vector registers):");
        let mut by_stmt: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        for (k, pass) in p.tier1.passes.iter().enumerate().filter(|(_, pass)| !pass.back) {
            let q = &pass.pressure;
            let hp = q.hot_int.len().max(1) as f64;
            eprintln!(
                "  pass {k}: hot loop {} points, peak {} integer, {} vector; over the registers at {:.0}% of its points on integer, {:.0}% on vector (whole function {} points, peak {}/{})",
                q.hot_int.len(),
                q.hot_int.iter().max().unwrap_or(&0),
                q.hot_vec.iter().max().unwrap_or(&0),
                100.0 * q.hot_int_over as f64 / hp,
                100.0 * q.hot_vec_over as f64 / hp,
                q.points,
                q.int_peak,
                q.vec_peak
            );
            // Which statements' values are live at the peak.
            let mut at: std::collections::BTreeMap<String, usize> = Default::default();
            for (o, made) in &q.at_peak {
                let o = *o;
                let stmt = o.and_then(|o| c.origin.get(o as usize)).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| if o.is_some() { "(lowering)".into() } else { format!("(unmarked {})", made.split(" of ").next().unwrap_or("")) });
                *at.entry(stmt).or_default() += 1;
            }
            let mut at: Vec<_> = at.into_iter().collect();
            at.sort_by_key(|a| std::cmp::Reverse(a.1));
            eprintln!("    live at the hot loop's peak, by statement: {}", at.iter().take(8).map(|(s, n)| format!("{n} {s}")).collect::<Vec<_>>().join("; "));
            // The hot body's pressure, first to last, in 16 stretches (each its most live).
            let curve = |v: &[usize]| -> String {
                let n = v.len().max(1);
                (0..16).map(|k| v[k * n / 16..((k + 1) * n / 16).max(k * n / 16 + 1).min(v.len())].iter().copied().max().unwrap_or(0).to_string()).collect::<Vec<_>>().join(" ")
            };
            if !q.hot_int.is_empty() {
                eprintln!("    {} integer values live through the hot loop; integer live, first to last: {}", q.hot_through, curve(&q.hot_int));
                if q.hot_vec.iter().any(|&v| v > 0) {
                    eprintln!("      vector live, first to last: {}", curve(&q.hot_vec));
                }
            }
            let loops: Vec<&super::super::native_cpu_pressure::Blame> = q.blame.iter().filter(|b| b.origin.is_none()).collect();
            let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
            for b in &loops {
                *kinds.entry(if b.made.starts_with("param") { "parameters" } else { b.made.as_str() }).or_default() += 1;
            }
            eprintln!("    the loop's own values over the registers: {} ({})", loops.len(), kinds.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", "));
            eprintln!("    most blamed of the body's values (points live while over the registers / points live):");
            for b in q.blame.iter().filter(|b| b.origin.is_some()).take(15) {
                eprintln!("    {:>6} / {:<6} {} {}", b.over, b.live, if b.vector { "v" } else { "x" }, name(b.origin, &b.made));
            }
            for b in &q.blame {
                let stmt = b.origin.and_then(|o| c.origin.get(o as usize)).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| if b.origin.is_some() { "(lowering)".into() } else { format!("(unmarked {})", b.made.split(" of ").next().unwrap_or("")) });
                let e = by_stmt.entry(stmt).or_default();
                e.0 += b.over;
                e.1 += 1;
            }
        }
        let mut by_stmt: Vec<_> = by_stmt.into_iter().collect();
        by_stmt.sort_by_key(|s| std::cmp::Reverse(s.1.0));
        eprintln!("  blame by statement, the hot loops of all passes (points live while over the registers, values):");
        for (s, (over, n)) in by_stmt.iter().take(15) {
            eprintln!("    {over:>7} {n:>5}  {s}");
        }
        if let Ok(dir) = std::env::var("RERAIL_RR_PROF_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).expect("the directory");
            for (name, text) in [("tier1.clif", &p.tier1.clif), ("tier1.s", &p.tier1.disassembly), ("tier2.clif", &p.tier2.clif), ("tier2.s", &p.tier2.disassembly)] {
                std::fs::write(dir.join(name), text).expect("writes");
            }
            for (k, pass) in p.tier1.passes.iter().enumerate() {
                std::fs::write(dir.join(format!("tier1.pass{k}.clif")), &pass.clif).expect("writes");
                std::fs::write(dir.join(format!("tier1.pass{k}.s")), &pass.disassembly).expect("writes");
            }
            std::fs::write(dir.join("tier1.rows"), &c.rows).expect("writes");
            std::fs::write(dir.join("tier2.words"), &c.words).expect("writes");
            std::fs::write(dir.join("tier2.machine"), &c.machine).expect("writes");
            eprintln!("written to {}", dir.display());
        }
        let load = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
        eprintln!("load: {load}");
    }
}
