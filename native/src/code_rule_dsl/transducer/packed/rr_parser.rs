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
use super::{native_cpu_stream, native_ir, rir};
use crate::code_rule_dsl::logic::rr;
use std::cell::RefCell;
use std::collections::HashMap;

/// The grammar: the ladder's last step.
const GRAMMAR: &str = include_str!("bitlex_ladder/rr/20-json-depth.bitlex");

/// Ids above the engine's, for the grammar's own names (supertypes, marks) that never reach a
/// tape.
const OWN_IDS: u16 = 0x4000;

/// The compiled parser, and which kinds are extras (comments).
pub(super) struct Compiled {
    pub(super) parser: native_cpu_stream::Parser,
    extra: Vec<u16>,
    /// Tier 1's cost by grammar statement, for the profile.
    pub(super) costs: Vec<bitlex_compile::Cost>,
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
    let extra = stmts
        .iter()
        .flat_map(|s| match s {
            ::bitlex::syntax::Stmt::Tree { body, .. } => body.clone(),
            _ => Vec::new(),
        })
        .filter_map(|s| match s {
            ::bitlex::syntax::Stmt::Extra(kinds) => Some(kinds),
            _ => None,
        })
        .flatten()
        .map(|k| id(rr::tape::KIND_NAMES, &k))
        .collect();
    Ok(Compiled { parser, extra, costs: compiled.costs.clone(), rows: compiled.rows.clone(), words: compiled.words.clone(), machine: compiled.machine.clone() })
}

/// `src`'s tape, as `logic::rr::tape` gives it: from the compiled grammar, or from the engine's
/// parser for a file the grammar refuses.
pub(in crate::code_rule_dsl) fn tape(src: &str) -> Vec<rr::tape::Node> {
    compiled_tape(src).unwrap_or_else(|| rr::tape(src))
}

/// The compiled grammar's tape, or `None` where it refuses the file (or did not compile).
pub(in crate::code_rule_dsl) fn compiled_tape(src: &str) -> Option<Vec<rr::tape::Node>> {
    let c = compiled().ok()?;
    let nodes = c.parser.parse(src.as_bytes()).ok()?;
    Some(nodes_to_tape(c, &nodes))
}

/// The compiled parser's nodes as the engine's tape nodes: each node's parent, from preorder and
/// sizes, and its extra flag.
pub(super) fn nodes_to_tape(c: &Compiled, nodes: &[super::stream::Node]) -> Vec<rr::tape::Node> {
    {
        // Preorder with sizes: each node's parent is the nearest open node before it.
        let mut out = Vec::with_capacity(nodes.len());
        let mut open: Vec<(u32, usize)> = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            while open.last().is_some_and(|&(_, end)| end <= i) {
                open.pop();
            }
            out.push(rr::tape::Node { kind: n.kind, field: n.field, start: n.start, end: n.end, parent: open.last().map(|o| o.0), extra: c.extra.contains(&n.kind) });
            open.push((i as u32, i + n.size as usize));
        }
        out
    }
}

/// Why the grammar did not compile, if it did not: a test's check.
#[cfg(test)]
pub(super) fn compile_error() -> Option<String> {
    compiled().err().map(str::to_owned)
}

#[cfg(test)]
mod tests {
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
            assert_eq!(super::tape(src), super::rr::tape(src), "{src:?}");
        }
        let deep = format!(".policy {{\"a\": {}{}}}", "[".repeat(127), "]".repeat(127));
        for src in ["r(x) :- x ~ ````a````.", deep.as_str(), "r(x) :- ."] {
            assert!(super::compiled_tape(src).is_none(), "the grammar takes {src:?}");
            assert_eq!(super::tape(src), super::rr::tape(src), "{src:?}");
        }
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
    fn corpus() -> Vec<String> {
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
    /// reloads, its branches and fixed points' loops. `RERAIL_RR_PROF_DIR`: each function's
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
        let mut stage = [std::time::Duration::ZERO; 6];
        let names = ["layout (Batch::new)", "tier 1 (masks, extracts)", "tape allocated", "tier 2 (words)", "nodes (Vec<Node>)", "to the engine's tape"];
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
                let nodes = tape.nodes();
                lap(4, &mut t);
                let out = super::nodes_to_tape(c, &nodes);
                lap(5, &mut t);
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
        eprintln!("tape rows: {room} allocated, {used} written ({:.0}x), {} MB zeroed a pass", room as f64 / used.max(1) as f64, room * 4 * 7 / 1_000_000);
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
