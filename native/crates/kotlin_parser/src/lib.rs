//! Kotlin's parser in Bitlex: the grammar of `bitlex_ladder/`'s last step, compiled once per
//! process through the Bitlex compiler to tier 1 and tier 2 on Cranelift.
//!
//! Its tape is tree-sitter-kotlin's, as the lean parser (`rerail_kotlin_lean`) writes it: the
//! grammar names the lean parser's kinds and fields, so a key here is the lean parser's key. A
//! file the compiled parser refuses (a construct the ladder does not reach yet, or any syntax
//! error) goes to the lean parser, and from there to tree-sitter.

use rerail_compiler::bitlex_masks as bitlex_compile;
use rerail_compiler::{native_cpu_clusters, native_cpu_stream, native_ir, rir_asm as rir};
use rerail_kotlin_lean as lean;

/// The grammar: the ladder's last step.
pub const GRAMMAR: &str = include_str!("../bitlex_ladder/25-class-delegates.bitlex");

/// The tape's tag: Kotlin's in the registry (`languages/kotlin.json`).
pub const TAG: u8 = 24;

/// The compiled parser.
pub struct Compiled {
    pub parser: native_cpu_stream::Parser,
    /// The program as assembled: the interpreter's input.
    pub program: rerail_compiler::program::Words,
    /// Tier 2's words and its state machine as text.
    pub words: String,
    pub machine: String,
}

// SAFETY: nothing changes a compiled parser once it is made (as `rerail_rr_parser::Compiled`):
// `image` takes `&self` and writes only buffers it allocates for the call.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

static PARSER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled parser, compiling it the first time.
pub fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(|| compile_text(GRAMMAR)).as_ref().map_err(String::as_str)
}

/// Ids above the lean parser's, for the grammar's own names (marks, flags) that never reach a
/// tape, as `rerail_rr_parser`'s.
const OWN_IDS: u16 = 0x4000;

/// A kind's id: the lean parser's, named first, then anonymous (`bitlex_masks::Symbols`).
fn lean_kind(name: &str) -> Option<u16> {
    if name == "ERROR" {
        return Some(u16::MAX);
    }
    let at = |named: bool| lean::KINDS.iter().position(|&(n, m)| n == name && m == named);
    at(true).or_else(|| at(false)).map(|i| i as u16)
}

fn field(name: &str) -> Option<u16> {
    let name = name.trim_start_matches("field ");
    lean::FIELDS.iter().position(|&f| f == name).filter(|&i| i > 0).map(|i| i as u16)
}

/// `grammar` compiled: a ladder step's text.
pub fn compile_text(grammar: &str) -> Result<Compiled, String> {
    let stmts = ::bitlex::syntax::parse(grammar)?;
    let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let kind = |name: &str| {
        lean_kind(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = OWN_IDS + own.len() as u16;
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let compiled = bitlex_compile::compile_shaped(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules)?;
    let program = rir::assemble(&compiled.text()).map_err(|e| format!("{e}\n{}", compiled.words))?;
    let mut native = native_ir::lower(&program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?;
    native.tag = TAG;
    for b in &mut native.block_programs {
        b.pad = rerail_compiler::native_cpu_block::BACK as u16;
    }
    let parser = native_cpu_stream::Parser::with_cut(&native, &native_cpu_clusters::by_rules(&native)).map_err(|e| format!("Cranelift: {e:?}"))?;
    Ok(Compiled { parser, program, words: compiled.words.clone(), machine: compiled.machine.clone() })
}

impl Compiled {
    /// What the program makes of `src`, for a test that fails: tier 1's tokens (start, last,
    /// code), the interpreter's answer, and the words around the one that refused.
    pub fn explain(&self, src: &str) -> String {
        let run = self.program.block_programs[0].run(src.as_bytes());
        let mut out = String::new();
        for i in 0..run.codes.len() {
            let at = |k: usize| run.positions.get(k).and_then(|p| p.get(i)).copied().unwrap_or(0);
            out.push_str(&format!("token {i}: {}..={} code {:#x}\n", at(0), at(1), run.codes[i]));
        }
        let cpu = self.parser.tier1.extract(src.as_bytes());
        for i in 0..cpu.codes.len() {
            let at = |k: usize| cpu.positions.get(k).and_then(|p| p.get(i)).copied().unwrap_or(0);
            out.push_str(&format!("cpu token {i}: {}..={} code {:#x}\n", at(0), at(1), cpu.codes[i]));
        }
        let interp = rerail_compiler::stream::run(&self.program, src.as_bytes(), &run);
        out.push_str(&format!("interpreter: {:?}\n", interp.as_ref().map(|n| n.len())));
        out.push_str(&format!("compiled tier 2 over the interpreter's tier 1: {:?}\n", self.parser.tier2.run(src.as_bytes(), &run).map(|n| n.len())));
        if let Err(why) = &interp {
            for line in self.words.lines().filter(|l| l.trim_start().split_whitespace().next().and_then(|w| w.parse::<usize>().ok()).is_some_and(|pc| pc + 6 >= why.pc && pc <= why.pc + 3)) {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// Where the interpreter refuses `src`: the first byte of the token it refused at.
    pub fn refused_at(&self, src: &str) -> Option<u32> {
        let run = self.program.block_programs[0].run(src.as_bytes());
        rerail_compiler::stream::run(&self.program, src.as_bytes(), &run).err().and_then(|r| r.at)
    }

    /// `src`'s tape image, or `None` where the grammar refuses the file.
    pub fn image(&self, src: &str) -> Option<Vec<u8>> {
        self.parser.image(src.as_bytes()).ok()
    }
}

/// A tape image as the lean parser's `tape_text` prints its tape: a node a line, indented by
/// depth, with its field, extra flag and span.
pub fn image_text(image: &[u8]) -> Result<String, String> {
    let v = rerail_tape_format::View::new(image)?;
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..v.len() {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let key = v.key(i);
        let (kind, field, extra) = (key & 0x3ff, key >> 10 & 0x3f, key >> 16 & 1 == 1);
        let name = lean::KINDS.get(kind as usize).map_or("?", |k| k.0);
        out.push_str(&format!(
            "{}{}{}{} [{}..{}]\n",
            "  ".repeat(open.len()),
            name,
            if field == 0 { String::new() } else { format!(" {}:", lean::FIELDS[field as usize]) },
            if extra { " flags=1" } else { "" },
            v.start(i),
            v.end(i)
        ));
        open.push(i + v.size(i));
    }
    Ok(out)
}

/// The lean parser's tape of `src` as text, or `None` where it refuses.
pub fn lean_text(src: &str) -> Option<String> {
    lean::tape(src.as_bytes()).ok().map(|t| lean::tape_text(&t))
}

#[cfg(test)]
mod tests;
