//! C's and C++'s directives in Bitlex (decision 0327, mode (a) compiled): the grammar of
//! `bitlex_ladder/`'s last step, compiled once a process through the Bitlex compiler to tier 1
//! and tier 2 on Cranelift, as `.rr`'s (`rerail_rr_parser`) and Kotlin's are.
//!
//! Its tape is the directive tape: a `directives` root and a node a directive, of its kind
//! (`KINDS`), from its `#` to its line's last token's end. Its rows (`rows`) are the lean scan's
//! (`rerail_c_pp::directives`), which is its reference. A file the grammar refuses (a construct
//! the ladder does not read yet) goes to the lean scan, so a row is never wrong.

use rerail_compiler::bitlex_masks as bitlex_compile;
use rerail_compiler::{native_cpu_clusters, native_cpu_stream, native_ir, rir_asm as rir};

/// The grammar: the ladder's last step.
pub const GRAMMAR: &str = include_str!("../bitlex_ladder/01-lines.bitlex");

/// The tape's kinds, by id: the root, the directives' kinds (their names the lean scan's
/// `Kind::name`), then the tokens' leaves.
pub const KINDS: &[&str] = &[
    "directives", "null", "unknown", "line_marker", "define", "undef", "include", "include_next", "import", "embed", "if", "ifdef", "ifndef", "elif", "elifdef",
    "elifndef", "else", "endif", "line", "error", "warning", "pragma", "ident", "sccs", "assert", "unassert", "hash", "identifier", "number", "punct", "string",
    "char_lit",
];

/// The first leaf's id: below it, a directive's kind.
const LEAVES: u16 = 26;

/// Ids above `KINDS`, for the grammar's own names (marks, keywords' leaves) that never name a
/// row, as `rerail_rr_parser`'s.
const OWN_IDS: u16 = 0x4000;

/// The compiled grammar.
pub struct Compiled {
    pub parser: native_cpu_stream::Parser,
    /// The same at 4-byte columns, for a file whose tape does not fit 2 (a tail past 64 KiB: a
    /// file's last directive far from its end). Decision 0140's guess, then page 37's wider parse.
    pub wide: native_cpu_stream::Parser,
    /// Tier 1's program as lowered, for its other executors (Metal).
    pub native: rerail_compiler::block_native::BlockNative,
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

/// The process's compiled grammar, compiled the first time.
pub fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(|| compile_text(GRAMMAR)).as_ref().map_err(String::as_str)
}

fn kind_id(name: &str) -> Option<u16> {
    if name == "ERROR" {
        return Some(u16::MAX);
    }
    KINDS.iter().position(|&k| k == name).map(|i| i as u16)
}

/// `grammar` compiled: a ladder step's text.
pub fn compile_text(grammar: &str) -> Result<Compiled, String> {
    let stmts = ::bitlex::syntax::parse(grammar)?;
    let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let kind = |name: &str| {
        kind_id(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = OWN_IDS + own.len() as u16;
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let field = |_: &str| None;
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let compiled = bitlex_compile::compile_shaped(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules)?;
    let program = rir::assemble(&compiled.text()).map_err(|e| format!("{e}\n{}", compiled.words))?;
    let mut native = native_ir::lower(&program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?;
    for b in &mut native.block_programs {
        b.pad = rerail_compiler::native_cpu_block::BACK as u16;
    }
    let cut = native_cpu_clusters::by_rules(&native);
    let parser = native_cpu_stream::Parser::with_cut(&native, &cut).map_err(|e| format!("Cranelift: {e:?}"))?;
    let widths = rerail_tape_format::Widths { size: 4, off: 4, tail: 4, from_start: false };
    let wide = native_cpu_stream::Parser {
        tier1: rerail_compiler::native_cpu_block::compile(&native.block_programs[0]).map_err(|e| format!("Cranelift: {e:?}"))?,
        tier2: native_cpu_stream::compile_at(&native, &cut, widths, native.tag).map_err(|e| format!("Cranelift: {e:?}"))?,
    };
    Ok(Compiled { parser, wide, native: native.block_programs[0].clone(), program, words: compiled.words.clone(), machine: compiled.machine.clone() })
}

/// A directive as the tape gives it: `c_directive`'s kind and bytes (the file's).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub kind: &'static str,
    pub start: u32,
    pub end: u32,
}

impl Compiled {
    /// `src`'s tape image, or `None` where the grammar refuses the file.
    pub fn image(&self, src: &[u8]) -> Option<Vec<u8>> {
        self.parser.image(src).or_else(|_| self.wide.image(src)).ok()
    }

    /// `src`'s directives, read from its tape in place, or `None` where the grammar refuses it.
    pub fn rows(&self, src: &[u8]) -> Option<Vec<Row>> {
        rows(&self.image(src)?).ok()
    }

    /// The same, by the interpreter: its tier 1 and its tier 2, the program's meaning.
    pub fn interpreter_rows(&self, src: &[u8]) -> Option<Vec<Row>> {
        let run = self.program.block_programs[0].run(src);
        let nodes = rerail_compiler::stream::run(&self.program, src, &run).ok()?;
        rows(&rerail_compiler::stream::image(&nodes, &self.program.readings.extras, 0, src)).ok()
    }

    /// The same for each of `files`, tier 1 on Metal and tier 2 over its buffers; `None` for a
    /// file refused, and for all where the device cannot run the batch.
    #[cfg(target_os = "macos")]
    pub fn metal_rows(&self, files: &[&[u8]]) -> Option<Vec<Option<Vec<Row>>>> {
        use rerail_metal::block_program::serial::{Segments, run_resident};
        let batch: Vec<_> = files.iter().map(|f| rerail_compiler::block::BatchFile { bytes: f, program: 0 }).collect();
        match run_resident(&[&self.native], &batch, Segments { blocks: 29, warm: 1 }) {
            Ok((_, run)) => Some((0..files.len()).map(|i| rerail_metal::tier2_on::image_on(&self.parser, &run, i).or_else(|_| rerail_metal::tier2_on::image_on(&self.wide, &run, i)).ok().and_then(|t| rows(&t).ok())).collect()),
            Err(rerail_compiler::Error::BackendCapacity) if files.len() > 1 => {
                let (a, b) = files.split_at(files.len() / 2);
                let mut out = self.metal_rows(a)?;
                out.extend(self.metal_rows(b)?);
                Some(out)
            }
            Err(_) => None,
        }
    }
}

/// The directives of a tape image: each node below the root of a directive's kind.
pub fn rows(image: &[u8]) -> Result<Vec<Row>, String> {
    let v = rerail_tape_format::View::new(image)?;
    let mut out = Vec::new();
    let mut i = 1;
    while i < v.len() {
        let kind = v.key(i) & 0x3ff;
        if kind >= LEAVES as u32 {
            return Err(format!("node {i}: {} below the root", KINDS.get(kind as usize).unwrap_or(&"?")));
        }
        out.push(Row { kind: KINDS[kind as usize], start: v.start(i) as u32, end: v.end(i) as u32 });
        i += v.size(i);
    }
    Ok(out)
}

/// A tape image as text: a node a line, indented by depth, with its span.
pub fn image_text(image: &[u8]) -> Result<String, String> {
    let v = rerail_tape_format::View::new(image)?;
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..v.len() {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let kind = v.key(i) & 0x3ff;
        out.push_str(&format!("{}{} [{}..{}]\n", "  ".repeat(open.len()), KINDS.get(kind as usize).unwrap_or(&"?"), v.start(i), v.end(i)));
        open.push(i + v.size(i));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
