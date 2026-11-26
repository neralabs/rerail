//! Python's parser in Bitlex: the grammar `python.bitlex`, compiled once per process through the
//! Bitlex compiler to tier 1 and tier 2 on Cranelift (decision 0698).
//!
//! Its tape is tree-sitter-python's, as the lean parser (`rerail_python_lean`) writes it: the
//! grammar names the lean parser's kinds, fields and supertypes, so a node here is the lean
//! parser's node. A file the compiled parser refuses (a construct the grammar does not read yet,
//! or any syntax error) goes to the lean parser, and from there to tree-sitter.

use rerail_bitlex::bitlex_masks as bitlex_compile;
use rerail_compiler::native_cpu_clusters;
use rerail_compiler::native_ir;
use rerail_cpu_stream::native_cpu_stream;
use rerail_python_lean as lean;
use rerail_rir::rir_asm as rir;

/// The grammar.
pub const GRAMMAR: &str = include_str!("../python.bitlex");

/// The tape's tag: Python's in the registry (`languages/python.json`).
pub const TAG: u8 = 1;

/// The compiled parser.
pub struct Compiled {
    pub parser: native_cpu_stream::Parser,
    /// The program as assembled: the interpreter's input.
    pub program: rerail_compiler::program::Words,
    /// Tier 2's words and its state machine as text.
    pub words: String,
    pub machine: String,
    /// The provers' answers the compile made, for the next (`compile_timed`'s `known`).
    pub proofs: Vec<(rerail_bitlex::bitlex_prove::Key, rerail_bitlex::bitlex_prove::Answer)>,
}

/// The provers' answers by question (decision 0414).
pub type Proofs = rerail_bitlex::bitlex_prove::Known;

// SAFETY: nothing changes a compiled parser once it is made (as `rerail_kotlin_parser::Compiled`):
// `image` takes `&self` and writes only buffers it allocates for the call.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

/// The grammar compiled, its provers' answers kept on the machine (`answers::answered`): what an
/// edit asks anew is proved by one compile, and kept for the next.
pub fn compile_answered(grammar: &str) -> Result<Compiled, String> {
    rerail_bitlex::answers::answered("python", grammar.as_bytes(), Proofs::default(), |known| {
        // `RERAIL_PYTHON_LAPS`: each phase's time, as it ends, on stderr.
        let laps = std::env::var_os("RERAIL_PYTHON_LAPS").is_some();
        let mut lap = |name: &'static str, d: std::time::Duration| {
            if laps {
                eprintln!("lap {name}: {d:?}");
            }
        };
        let out = kept(grammar, known, &mut lap)?;
        let made = out.proofs.clone();
        Ok((from_output(out, &mut lap)?, made))
    })
}

static PARSER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled parser, compiling it the first time ([`compile_answered`]).
pub fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(|| compile_answered(GRAMMAR)).as_ref().map_err(String::as_str)
}

/// Ids above the lean parser's, for the grammar's own names (marks, flags) that never reach a
/// tape, as `rerail_kotlin_parser`'s.
const OWN_IDS: u16 = 0x4000;

/// The supertypes the lean parser's keys name (`lean::CLASSES`), each an id of its own below
/// [`OWN_IDS`]: a supertype is no kind of the lean parser's, but the tape's code table names it.
const SUPERTYPES: &[&str] = &["expression", "primary_expression", "pattern", "parameter"];
const SUPERTYPE_IDS: u16 = 0x3000;

/// A kind's id: the lean parser's, named first, then anonymous; a supertype's own.
fn lean_kind(name: &str) -> Option<u16> {
    if name == "ERROR" {
        return Some(u16::MAX);
    }
    if let Some(i) = SUPERTYPES.iter().position(|&s| s == name) {
        return Some(SUPERTYPE_IDS + i as u16);
    }
    let at = |named: bool| lean::KINDS.iter().position(|&(n, m)| n == name && m == named);
    at(true).or_else(|| at(false)).map(|i| i as u16)
}

fn field(name: &str) -> Option<u16> {
    let name = name.trim_start_matches("field ");
    lean::FIELDS.iter().position(|&f| f == name).filter(|&i| i > 0).map(|i| i as u16)
}

/// `grammar` compiled: a Bitlex grammar's text.
pub fn compile_text(grammar: &str) -> Result<Compiled, String> {
    compile_timed(grammar, Default::default(), &mut |_, _| {})
}

/// `compile_text`, each phase's time given to `lap` as it ends, as `rerail_kotlin_parser`'s.
/// `known`: the provers' answers a compile made before (`Compiled::proofs`, decision 0414).
pub fn compile_timed(grammar: &str, known: Proofs, lap: &mut dyn FnMut(&'static str, std::time::Duration)) -> Result<Compiled, String> {
    let out = bitlex_output(grammar, known, lap)?;
    from_output(out, lap)
}

/// What the Bitlex compiler makes of a grammar that the rest of a compile reads (`from_output`): the program's text,
/// tier 2's words and its machine as text, and the provers' answers it made.
pub struct Output {
    pub text: String,
    pub words: String,
    pub machine: String,
    pub proofs: Vec<(rerail_bitlex::bitlex_prove::Key, rerail_bitlex::bitlex_prove::Answer)>,
}

/// `bitlex_output`, kept for this test binary's other processes (`answers::kept_output`): its first process compiles
/// the grammar, the others read what it made.
fn kept(grammar: &str, known: Proofs, lap: &mut dyn FnMut(&'static str, std::time::Duration)) -> Result<Output, String> {
    let bytes = rerail_bitlex::answers::kept_output("python-compiled", grammar.as_bytes(), || {
        let o = bitlex_output(grammar, known, lap)?;
        Ok(rerail_bitlex::answers::pack(&[o.text.as_bytes(), o.words.as_bytes(), o.machine.as_bytes(), &rerail_bitlex::bitlex_prove::encode(&o.proofs)]))
    })?;
    let parts = rerail_bitlex::answers::unpack(&bytes, 4).ok_or("a kept compile not of four parts")?;
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|e| e.to_string());
    Ok(Output { text: text(parts[0])?, words: text(parts[1])?, machine: text(parts[2])?, proofs: rerail_bitlex::bitlex_prove::decode(parts[3]).into_iter().collect() })
}

/// The Bitlex compiler's half of a compile (`compile_timed`): the grammar's syntax and the compiler.
pub fn bitlex_output(grammar: &str, known: Proofs, lap: &mut dyn FnMut(&'static str, std::time::Duration)) -> Result<Output, String> {
    let mut at = std::time::Instant::now();
    let mut lap = |name: &'static str| {
        lap(name, at.elapsed());
        at = std::time::Instant::now();
    };
    let stmts = ::bitlex::syntax::parse(grammar)?;
    lap("syntax");
    let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let kind = |name: &str| {
        lean_kind(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = OWN_IDS + own.len() as u16;
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field, terminators: rerail_tape_format::newlines::Terminators::Newlines, rust: false };
    let compiled = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, known)?;
    lap("bitlex compiler");
    Ok(Output { text: compiled.text(), words: compiled.words.clone(), machine: compiled.machine.clone(), proofs: compiled.proofs.clone() })
}

/// The rest of a compile, from the Bitlex compiler's output: the words assembled, lowered to the native IR, tier 2's
/// functions cut into clusters, and Cranelift's code for each tier.
pub fn from_output(compiled: Output, lap: &mut dyn FnMut(&'static str, std::time::Duration)) -> Result<Compiled, String> {
    let mut at = std::time::Instant::now();
    let mut lap = |name: &'static str| {
        lap(name, at.elapsed());
        at = std::time::Instant::now();
    };
    let program = rir::assemble(&compiled.text).map_err(|e| format!("{e}\n{}", compiled.words))?;
    lap("assembled");
    let mut native = native_ir::lower(&program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?;
    lap("native IR");
    native.tag = TAG;
    for b in &mut native.block_programs {
        b.pad = rerail_cpu_block::native_cpu_block::BACK as u16;
    }
    let cut = native_cpu_clusters::by_rules(&native);
    lap("clusters");
    let block = native.block_programs.first().ok_or("no tier 1 program")?;
    let tier1 = rerail_cpu_block::native_cpu_block::compile(block).map_err(|e| format!("Cranelift: {e:?}"))?;
    lap("tier 1 Cranelift");
    let tier2 = native_cpu_stream::compile_cut(&native, &cut).map_err(|e| format!("Cranelift: {e:?}"))?;
    lap("tier 2 Cranelift");
    let parser = native_cpu_stream::Parser::with_tiers(&native, &cut, tier1, tier2);
    Ok(Compiled { parser, program, words: compiled.words, machine: compiled.machine, proofs: compiled.proofs })
}

/// The parser as Rust (`rerail-python-aot`): tier 1 from the lexer's statements, tier 2 from the tree rules, or why
/// the compiler writes neither. A compile of its own, as `rerail_kotlin_parser::tiers_rust`: the parser's
/// (`compiled`) writes no Rust.
pub fn tiers_rust(grammar: &str) -> Result<(String, String), String> {
    rerail_bitlex::answers::answered("python", grammar.as_bytes(), Proofs::default(), |known| {
        let stmts = ::bitlex::syntax::parse(grammar)?;
        let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
        let kind = |name: &str| {
            lean_kind(name).or_else(|| {
                let mut own = own.borrow_mut();
                let next = OWN_IDS + own.len() as u16;
                Some(*own.entry(name.to_owned()).or_insert(next))
            })
        };
        let symbols = bitlex_compile::Symbols { kind: &kind, field: &field, terminators: rerail_tape_format::newlines::Terminators::Newlines, rust: true };
        let compiled = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, known)?;
        let tiers = match (compiled.lexer_rust, compiled.rust) {
            (Some(Ok(t1)), Some(Ok(t2))) => Ok((t1, t2)),
            (t1, t2) => Err(format!("tier 1: {:?}; tier 2: {:?}", t1.map(|x| x.map(|_| ())), t2.map(|x| x.map(|_| ())))),
        };
        Ok((tiers, compiled.proofs))
    })?
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
        // What the reference evaluator refuses the file for: the grammar's escapes and errors.
        if let Ok(stmts) = ::bitlex::syntax::parse(GRAMMAR) {
            match ::bitlex::eval::Ev::run(&stmts, src.as_bytes(), &Default::default()) {
                Ok(e) => e.errors.iter().for_each(|e| out.push_str(&format!("tier 1: {e}\n"))),
                Err(e) => out.push_str(&format!("tier 1's reference: {e:?}\n")),
            }
        }
        let interp = rerail_compiler::stream::run(&self.program, src.as_bytes(), &run);
        out.push_str(&format!("interpreter: {:?}\n", interp.as_ref().map(|n| n.len())));
        if let Err(why) = &interp {
            out.push_str(&format!("refused: {why:?}\n"));
            for line in self.words.lines().filter(|l| l.split_whitespace().next().and_then(|w| w.parse::<usize>().ok()).is_some_and(|pc| pc + 6 >= why.pc && pc <= why.pc + 3)) {
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
/// depth, its kind, field, extra flag, supertypes and span.
pub fn image_text(image: &[u8]) -> Result<String, String> {
    let v = rerail_tape_format::View::new(image)?;
    let (columns, table) = rerail_tape_format::tape::f15_columns(image)?;
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..v.len() {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let code = u16::from_le_bytes([columns.code[2 * i], columns.code[2 * i + 1]]);
        let e = table.entry(u32::from(code));
        let name = lean::KINDS.get(usize::from(e.kind)).map_or("?", |k| k.0);
        let mut classes: Vec<&str> = e.classes.iter().map(|&c| c.checked_sub(SUPERTYPE_IDS).and_then(|j| SUPERTYPES.get(usize::from(j))).copied().unwrap_or("?")).collect();
        classes.sort_unstable();
        out.push_str(&format!(
            "{}{}{}{}{} [{}..{}]\n",
            "  ".repeat(open.len()),
            name,
            if e.field == 0 { String::new() } else { format!(" {}:", lean::FIELDS.get(usize::from(e.field)).copied().unwrap_or("?")) },
            if e.flags & rerail_tape_format::CODE_EXTRA != 0 { " flags=1" } else { "" },
            if classes.is_empty() { String::new() } else { format!(" {{{}}}", classes.join(",")) },
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
