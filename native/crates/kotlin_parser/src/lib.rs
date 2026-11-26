//! Kotlin's parser in Bitlex: the grammar `kotlin.bitlex`, compiled once per
//! process through the Bitlex compiler to tier 1 and tier 2 on Cranelift.
//!
//! Its tape is tree-sitter-kotlin's, as the lean parser (`rerail_kotlin_lean`) writes it: the
//! grammar names the lean parser's kinds and fields, so a key here is the lean parser's key. A
//! file the compiled parser refuses (a construct the grammar does not read yet, or any syntax
//! error) goes to the lean parser, and from there to tree-sitter.

use rerail_bitlex::bitlex_masks as bitlex_compile;
use rerail_compiler::native_ir;
use rerail_rir::rir_asm as rir;
use rerail_compiler::native_cpu_clusters;
use rerail_cpu_stream::native_cpu_stream;
use rerail_kotlin_lean as lean;

/// The grammar.
pub const GRAMMAR: &str = include_str!("../kotlin.bitlex");

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
    /// The provers' answers the compile made, for the next (`compile_timed`'s `known`).
    pub proofs: Vec<(rerail_bitlex::bitlex_prove::Key, rerail_bitlex::bitlex_prove::Answer)>,
    /// Kotlin's numbers by tier 2's keys, made at the first tape.
    numbers: std::sync::OnceLock<native_cpu_stream::Numbers>,
}

/// The tape: format 16, in Kotlin's table (decision 0631).
pub type Tape = rerail_tape_format::f16::Tape16;

/// Kotlin's format 16 table.
pub fn lang() -> &'static rerail_tape_format::f16::Lang {
    rerail_tape_format::f16::lang(TAG).expect("Kotlin's format 16 table")
}

/// A key's number in Kotlin's table, as the lean parser's sink numbers it (`lean::sink`, by its names): the grammar's
/// kinds and fields are the lean parser's, so its keys are too.
fn number(kind: u16, field: u16, classes: &[u16], extra: bool) -> Option<u32> {
    let key = u32::from(kind) | u32::from(field) << rerail_tape_format::KIND_BITS | u32::from(extra) << 16;
    if !classes.is_empty() || field >= 1 << 6 {
        return None;
    }
    lang().number_named(&lean::names(key)?)
}

/// A format 16 tape (16z is never asked for here).
fn sixteen(w: rerail_tape_format::f16::Written) -> Result<Tape, String> {
    match w {
        rerail_tape_format::f16::Written::F16(t) => Ok(t),
        rerail_tape_format::f16::Written::F16Z(_) => Err("a 16z tape where format 16 was asked for".into()),
    }
}

/// The provers' answers by question (decision 0414).
pub type Proofs = rerail_bitlex::bitlex_prove::Known;

// SAFETY: nothing changes a compiled parser once it is made (as `rerail_rr_parser::Compiled`):
// `image` takes `&self` and writes only buffers it allocates for the call.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

/// The provers' answers to the grammar's questions, shipped beside it (`kotlin.answers`, in
/// `bitlex_prove::encode`'s bytes: a 16-byte digest a question, 52 KB): decision 0414's seed (Q234).
/// A compile given them proves only what an edit asks anew: 41 s cold, 1.5 s with them. Answers of
/// another provers' version are none (`decode`), so the seed is never wrong, only slow.
/// `tests::the_shipped_answers_answer_the_grammar` says when it lacks one, and writes it again
/// (`RERAIL_BLESS_ANSWERS=1`, `answers::check_shipped`).
pub const ANSWERS: &[u8] = include_bytes!("../kotlin.answers");

/// [`ANSWERS`], decoded: `compile_timed`'s `known`.
pub fn shipped_answers() -> Proofs {
    rerail_bitlex::bitlex_prove::decode(ANSWERS)
}

/// The grammar compiled from the shipped answers and the machine's (`answers::answered`: what an
/// edit asks anew is proved by one compile on the machine, and kept for the others).
pub fn compile_answered(grammar: &str) -> Result<Compiled, String> {
    rerail_bitlex::answers::answered("kotlin", grammar.as_bytes(), shipped_answers(), |known| {
        let c = compile_timed(grammar, known, &mut |_, _| {})?;
        let made = c.proofs.clone();
        Ok((c, made))
    })
}

static PARSER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled parser, compiling it the first time ([`compile_answered`]).
pub fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(|| compile_answered(GRAMMAR)).as_ref().map_err(String::as_str)
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

/// Tier 1 and tier 2 of `grammar` as Rust, as the Bitlex compiler writes them from the lexer's
/// statements (`bitlex_lex_rust`) and the tree rules (`bitlex_rust`), without either tier
/// compiled. The provers start from the shipped answers ([`ANSWERS`]), as every other compile of the grammar.
pub fn tiers_rust(grammar: &str) -> Result<(String, String), String> {
    tiers_rust_given(grammar, shipped_answers()).map(|(t1, t2, _)| (t1, t2))
}

/// [`tiers_rust`] from the answers `known`, and the answers its compile made: the Rust's compile
/// asks what the grammar's (`compile_timed`) does not, with the symbols and tier 2's shape, so
/// the shipped answers hold both (`tests::the_shipped_answers_answer_the_grammar`).
pub fn tiers_rust_given(grammar: &str, known: Proofs) -> Result<(String, String, Vec<(rerail_bitlex::bitlex_prove::Key, rerail_bitlex::bitlex_prove::Answer)>), String> {
    let stmts = ::bitlex::syntax::parse(grammar)?;
    let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let kind = |name: &str| {
        lean_kind(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = OWN_IDS + own.len() as u16;
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field, terminators: rerail_tape_format::newlines::Terminators::Newlines };
    // The shipped answers: the compile's own, so the same Rust, with no cold proofs.
    let compiled = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, known, bitlex_compile::AsRust::Yes)?;
    let tier1 = compiled.lexer_rust.ok_or_else(|| "no tree section".to_owned())??;
    let tier2 = compiled.rust.ok_or_else(|| "no tree section".to_owned())??;
    Ok((tier1, tier2, compiled.proofs))
}

/// `grammar` compiled: a Bitlex grammar's text.
pub fn compile_text(grammar: &str) -> Result<Compiled, String> {
    compile_timed(grammar, Default::default(), &mut |_, _| {})
}

/// `compile_text`, each phase's time given to `lap` as it ends: Bitlex's syntax, the Bitlex
/// compiler (tier 1's masks and tier 2's words), the words assembled, lowered to the native IR,
/// tier 2's functions cut into clusters, and Cranelift's code for each tier. `known`: the provers'
/// answers a compile made before (`Compiled::proofs`, decision 0414).
pub fn compile_timed(grammar: &str, known: Proofs, lap: &mut dyn FnMut(&'static str, std::time::Duration)) -> Result<Compiled, String> {
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
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field, terminators: rerail_tape_format::newlines::Terminators::Newlines };
    let compiled = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, known, bitlex_compile::AsRust::No)?;
    lap("bitlex compiler");
    let program = rir::assemble(&compiled.text()).map_err(|e| format!("{e}\n{}", compiled.words))?;
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
    // Tier 2 at the stride of its own tier 1's records, compiled in (decision 0696).
    let cut = native_cpu_clusters::Cut { stride: Some(native_cpu_stream::cpu_stride(&tier1)), ..cut };
    let tier2 = native_cpu_stream::compile_cut(&native, &cut).map_err(|e| format!("Cranelift: {e:?}"))?;
    lap("tier 2 Cranelift");
    let parser = native_cpu_stream::Parser::with_tiers(&native, &cut, tier1, tier2);
    Ok(Compiled { parser, program, words: compiled.words.clone(), machine: compiled.machine.clone(), proofs: compiled.proofs.clone(), numbers: Default::default() })
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
        #[cfg(feature = "test-hooks")]
        {
            let tape = self.parser.tier2.image16_on(src.as_bytes(), lean::TERMINATORS, &run, lang(), self.numbers(), false);
            out.push_str(&format!("compiled tier 2 over the interpreter's tier 1: {:?}\n", tape.map(|w| w.extent().len())));
        }
        if let Err(why) = &interp {
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

    /// The numbers by tier 2's keys, made once.
    fn numbers(&self) -> &native_cpu_stream::Numbers {
        self.numbers.get_or_init(|| self.parser.tier2.numbers_by(&number))
    }

    /// `src`'s tape in format 16 as tier 2 writes it, or why the grammar refuses the file.
    pub fn tape(&self, src: &str) -> Result<Tape, String> {
        sixteen(self.parser.image16(src.as_bytes(), lean::TERMINATORS, lang(), self.numbers(), false)?)
    }

    /// `src`'s tape, or `None` where the grammar refuses the file.
    pub fn image(&self, src: &str) -> Option<Tape> {
        self.tape(src).ok()
    }

    /// `src`'s tape by the interpreter (its tier 1 and its tier 2, the program's meaning), or why it refuses it.
    pub fn interpreted(&self, src: &str) -> Result<Tape, String> {
        let run = self.program.block_programs[0].run(src.as_bytes());
        let nodes = rerail_compiler::stream::run(&self.program, src.as_bytes(), &run).map_err(|why| format!("{why:?}"))?;
        sixteen(rerail_compiler::stream::image16_by(&nodes, &self.program.readings.extras, &number, rerail_compiler::stream::Form16 { lang: lang(), terminators: lean::TERMINATORS, tag: TAG, z: false }, src.as_bytes())?)
    }
}

/// A tape as the lean parser's `tape_text` prints its tape (`tape::plain_text`, by Kotlin's names): a node a line,
/// indented by depth, its kind, field, flags and span.
pub fn image_text(tape: &Tape) -> Result<String, String> {
    Ok(rerail_tape_format::tape::plain_text(&tape.reader(lang())?, lang().names()))
}

/// The lean parser's tape of `src` as text, or `None` where it refuses.
pub fn lean_text(src: &str) -> Option<String> {
    lean::tape(src.as_bytes()).ok().map(|t| lean::tape_text(&t))
}

#[cfg(test)]
mod tests;

/// Texts with a block comment that starts a line before a token that goes on the line before:
/// lost where the lean parser asks for a semicolon there, any comment inside brackets (decision
/// 0451). This crate's tests check them against the lean parser, `kotlin_aot`'s against the JIT.
#[doc(hidden)]
pub fn lost_comment_texts() -> Vec<String> {
    let bodies = [
        "val a = b\n            /* c */ .d()",
        "val a = b\n            /* c */ ?: d",
        "val a = b\n            /* c */ as String",
        "val a = b\n            /* c */ + d",
        "foo(b\n            /* c */ .d)",
        "val a = listOf(1)\n            /* c */ .map { it }\n            /* e */ .size",
        "val a = b\n            /* c */ != d",
        "if (b\n            /* c */ .d) return 1",
        "return b\n            /* c */ .d",
        "b\n            /* c */ .d()",
        "val a = b /* c */\n            .d()",
        "val a = b\n            /* c */\n            .d()",
        "a\n        /* c */ .b()",
    ];
    bodies.iter().map(|b| format!("class A {{\n    fun f(b: Any?): Int {{\n        {b}\n        return 0\n    }}\n}}\n")).collect()
}
