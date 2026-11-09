//! The `.rr` parser: the Bitlex grammar of `.rr` (the last step of `bitlex_ladder/rr/`), compiled
//! once per thread through the Bitlex compiler to tier 1 and tier 2 on Cranelift.
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
struct Compiled {
    parser: native_cpu_stream::Parser,
    extra: Vec<u16>,
}

thread_local! {
    static PARSER: Result<Compiled, String> = compile();
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
    Ok(Compiled { parser, extra })
}

/// `src`'s tape, as `logic::rr::tape` gives it: from the compiled grammar, or from the engine's
/// parser for a file the grammar refuses.
pub(in crate::code_rule_dsl) fn tape(src: &str) -> Vec<rr::tape::Node> {
    compiled_tape(src).unwrap_or_else(|| rr::tape(src))
}

/// The compiled grammar's tape, or `None` where it refuses the file (or did not compile).
pub(in crate::code_rule_dsl) fn compiled_tape(src: &str) -> Option<Vec<rr::tape::Node>> {
    PARSER.with(|p| {
        let c = p.as_ref().ok()?;
        let nodes = c.parser.parse(src.as_bytes()).ok()?;
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
        Some(out)
    })
}

/// Why the grammar did not compile, if it did not: a test's check.
#[cfg(test)]
pub(super) fn compile_error() -> Option<String> {
    PARSER.with(|p| p.as_ref().err().cloned())
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
}
