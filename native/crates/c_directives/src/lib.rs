//! C's and C++'s directives in Bitlex (decision 0327, mode (a) compiled): the grammar
//! `directives.bitlex`, compiled once a process through the Bitlex compiler to tier 1
//! and tier 2 on Cranelift, as `.rerail`'s (`rerail_rr_parser`) and Kotlin's are.
//!
//! Its tape is the directive tape: a `directives` root and a node a directive, of its kind
//! (`KINDS`), from its `#` to its line's last token's end, its operands fields' nodes below it.
//! Its rows (`rows`) and operands (`operands`) are the lean scan's (`rerail_c_pp::directives`),
//! which is its reference. A file the grammar refuses (a construct
//! it does not read) goes to the lean scan, so a row is never wrong.

use rerail_bitlex::bitlex_masks as bitlex_compile;
use rerail_bitlex::bitlex_prove::{Answer, Key, Known};
use rerail_compiler::native_ir;
use rerail_rir::rir_asm as rir;
use rerail_compiler::native_cpu_clusters;
use rerail_cpu_stream::native_cpu_stream;

/// A grammar lowered (`lower_kinds`): the assembled program, the native IR, tier 2's words and its
/// state machine as text, and how many of its names it numbered from `OWN_IDS`.
pub(crate) type Lowered = (rerail_compiler::program::Words, native_ir::Structure, String, String, usize);

/// The grammar.
pub const GRAMMAR: &str = include_str!("../directives.bitlex");

/// The provers' answers to its questions, shipped beside it (decision 0669).
pub const ANSWERS: &[u8] = include_bytes!("../directives.answers");

/// A grammar's shipped answers (decision 0669): `<grammar>.answers`' bytes, and the family the
/// machine keeps the answers its compiles make under (`answers::answered`), one a text.
#[derive(Clone, Copy)]
pub(crate) struct Seed {
    pub family: &'static str,
    pub shipped: &'static [u8],
}

/// The directives' answers.
const SEED: Seed = Seed { family: "c-directives", shipped: ANSWERS };

/// The tape's kinds, by id: the root, the directives' kinds (their names the lean scan's
/// `Kind::name`), then the tokens' leaves and the operands' nodes.
pub const KINDS: &[&str] = &[
    "directives", "null", "unknown", "line_marker", "define", "undef", "include", "include_next", "import", "embed", "if", "ifdef", "ifndef", "elif", "elifdef",
    "elifndef", "else", "endif", "line", "error", "warning", "pragma", "ident", "sccs", "assert", "unassert", "hash", "identifier", "number", "punct", "string",
    "char_lit", "fn_open", "ellipsis", "header", "params", "operands", "plain_body", "macro_header", "pragma_text", "line_rest", "marker_rest", "group", "code",
];

/// The operands' fields, by id less one (0: no field).
pub const FIELDS: &[&str] = &["name", "params", "param", "variadic", "body", "spelled", "expr", "text", "first", "second", "n", "file", "rest"];

/// The first leaf's id: below it, a directive's kind.
const LEAVES: u16 = 26;

/// Ids above `KINDS`, for the grammar's own names (marks, keywords' leaves, the operands'
/// nodes). Below 0x400: a tape's kind is 10 bits (`rerail_tape_format::KIND_BITS`), and an id
/// past them would read as another kind, its field lost.
const OWN_IDS: u16 = 0x300;

/// The compiled grammar.
pub struct Compiled {
    pub parser: native_cpu_stream::Parser,
    /// Tier 1's program as lowered, for its other executors (Metal).
    pub native: rerail_compiler::block_native::BlockNative,
    /// The program as assembled: the interpreter's input.
    pub program: rerail_compiler::program::Words,
    /// Tier 2's words and its state machine as text.
    pub words: String,
    pub machine: String,
    /// The tape's format 16 table (decision 0631): the preprocessor tape's, or one of the grammar's names.
    table: Table,
    /// The table's numbers by tier 2's keys, made at the first tape.
    numbers: std::sync::OnceLock<native_cpu_stream::Numbers>,
}

/// A compiled grammar's format 16 table: one shared with another writer of its tapes (the preprocessor tape's,
/// `rerail_c_pp::tape::lang`), or its own, of its names (`table`).
enum Table {
    Shared(&'static rerail_tape_format::f16::Lang),
    Own(rerail_tape_format::f16::Lang),
}

/// The format 16 table of a grammar's tape (decision 0631): each of its kinds (the `kinds` given, then the `own`
/// names numbered from `OWN_IDS`) with no field, then with each of its `fields`; an extra's (`extras`) flagged one.
/// Its numbers are its entries' places, the kinds with no field first: most of a tape's nodes.
fn table(kinds: usize, own: usize, fields: usize, extras: &[u16]) -> rerail_tape_format::f16::Lang {
    use rerail_tape_format::tape::{CodeTable, Entry};
    let ids: Vec<u16> = (0..kinds as u16).chain(OWN_IDS..OWN_IDS + own as u16).collect();
    let entry = |kind: u16, field: u16| Entry { kind, field, flags: if extras.contains(&kind) { rerail_tape_format::CODE_EXTRA } else { 0 }, alias: None, classes: Vec::new() };
    let entries = (0..=fields as u16).flat_map(|f| ids.iter().map(move |&k| entry(k, f))).collect();
    rerail_tape_format::f16::Lang::new(CodeTable::new(entries), [1, 1, 1], 1).expect("a grammar's tape table")
}

/// The tapes' tag and their lines' terminators: the preprocessor tape's (`rerail_c_pp::tape`), C's.
const TAG: u8 = rerail_c_pp::tape::TAG;
const TERMINATORS: rerail_tape_format::newlines::Terminators = rerail_c_pp::tape::LINE_ENDS;

/// A format 16 tape (16z is never asked for here).
fn written(w: rerail_tape_format::f16::Written) -> Option<Tape16> {
    match w {
        rerail_tape_format::f16::Written::F16(t) => Some(t),
        rerail_tape_format::f16::Written::F16Z(_) => None,
    }
}

/// A compiled grammar's tape: format 16, with each block's presence.
pub type Tape16 = rerail_tape_format::f16::Tape16;

/// A tape read in place through format 16's reads (`f16::F16`): each node's key (its code's kind and field, `kind |
/// field << KIND_BITS`), span and subtree.
pub struct Nodes<'a>(rerail_tape_format::f16::F16<'a>);

/// Where two tapes of one table differ: their first node of another key, span or subtree, or else their bytes'
/// first difference (the line index, the presence). For a test's message.
pub fn difference(a: &Tape16, b: &Tape16, lang: &rerail_tape_format::f16::Lang) -> String {
    let (Ok(x), Ok(y)) = (Nodes::of(a, lang), Nodes::of(b, lang)) else { return "a tape that does not read".into() };
    let node = |v: &Nodes<'_>, i: usize| (i < v.len()).then(|| (v.key(i), v.start(i), v.end(i), v.size(i)));
    if let Some(i) = (0..x.len().max(y.len())).find(|&i| node(&x, i) != node(&y, i)) {
        return format!("node {i} of {} / {}: {:?} / {:?}", x.len(), y.len(), node(&x, i), node(&y, i));
    }
    let (p, q) = (a.as_bytes(), b.as_bytes());
    let k = p.iter().zip(q).position(|(u, v)| u != v).unwrap_or(p.len().min(q.len()));
    format!("the same {} nodes; the bytes differ at {k} of {} / {}", x.len(), p.len(), q.len())
}

impl<'a> Nodes<'a> {
    pub fn of(t: &'a Tape16, lang: &'a rerail_tape_format::f16::Lang) -> Result<Self, String> {
        Ok(Nodes(t.reader(lang)?))
    }
    pub fn len(&self) -> usize {
        use rerail_tape_format::tape::Tape as _;
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn key(&self, i: usize) -> u32 {
        use rerail_tape_format::tape::Tape as _;
        let e = self.0.table().entry(self.0.code(i));
        u32::from(e.kind) | u32::from(e.field) << rerail_tape_format::KIND_BITS
    }
    pub fn start(&self, i: usize) -> u32 {
        use rerail_tape_format::tape::Tape as _;
        self.0.start(i) as u32
    }
    pub fn end(&self, i: usize) -> u32 {
        use rerail_tape_format::tape::Tape as _;
        self.0.end(i) as u32
    }
    /// Its subtree's nodes, itself counted.
    pub fn size(&self, i: usize) -> usize {
        use rerail_tape_format::tape::Tape as _;
        self.0.subtree_end(i) - i
    }
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

static LOWERED: std::sync::OnceLock<Result<(rerail_compiler::program::Words, rerail_tape_format::f16::Lang), String>> = std::sync::OnceLock::new();

/// `src`'s directives by the interpreter, the grammar lowered once a process (no Cranelift), or
/// `None` where the grammar refuses it.
pub fn interpreted_read(src: &[u8]) -> Option<Read> {
    let (program, lang) = LOWERED.get_or_init(|| lower_kinds(GRAMMAR, KINDS, FIELDS, SEED).map(|l| { let lang = table(KINDS.len(), l.4, FIELDS.len(), &l.0.readings.extras); (l.0, lang) })).as_ref().ok()?;
    let run = program.block_programs[0].run(src);
    let nodes = rerail_compiler::stream::run(program, src, &run).ok()?;
    let tape = written(rerail_compiler::stream::image16(&nodes, &program.readings.extras, lang, TERMINATORS, TAG, src).ok()?)?;
    read(&Nodes::of(&tape, lang).ok()?).ok()
}

/// `grammar` compiled: a grammar's text.
pub fn compile_text(grammar: &str) -> Result<Compiled, String> {
    compile_kinds(grammar, KINDS, FIELDS, SEED, None)
}

/// `grammar` compiled with its tape's kinds and fields by id (`kinds`, `fields`: a field's id is
/// its index plus one), the grammar's other names numbered from `OWN_IDS`, from its shipped answers (`seed`,
/// decision 0669). Its tapes are numbered in `shared` where given (another writer's table of the same names), else
/// in a table of its names (`table`).
pub(crate) fn compile_kinds(grammar: &str, kinds: &[&str], fields: &[&str], seed: Seed, shared: Option<&'static rerail_tape_format::f16::Lang>) -> Result<Compiled, String> {
    let (program, native, words, machine, own) = lower_kinds(grammar, kinds, fields, seed)?;
    let cut = native_cpu_clusters::by_rules(&native);
    let parser = native_cpu_stream::Parser::with_cut(&native, &cut).map_err(|e| format!("Cranelift: {e:?}"))?;
    let table = match shared {
        Some(lang) => Table::Shared(lang),
        None => Table::Own(table(kinds.len(), own, fields.len(), &program.readings.extras)),
    };
    Ok(Compiled { parser, native: native.block_programs[0].clone(), program, words, machine, table, numbers: Default::default() })
}

/// `grammar` through the Bitlex compiler, assembled and lowered to the native IR: all but
/// Cranelift's compile (`compile_kinds`), and how many of the grammar's names it numbered from `OWN_IDS`; from the
/// grammar's shipped answers and the machine's (decision 0669).
pub(crate) fn lower_kinds(grammar: &str, kinds: &[&str], fields: &[&str], seed: Seed) -> Result<Lowered, String> {
    rerail_bitlex::answers::answered_shipped(seed.family, grammar.as_bytes(), seed.shipped, |known| lower_known(grammar, kinds, fields, known))
}

/// `lower_kinds` from the provers' answers `known`, and the answers its compile made.
pub(crate) fn lower_known(grammar: &str, kinds: &[&str], fields: &[&str], known: Known) -> Result<(Lowered, Vec<(Key, Answer)>), String> {
    let kind_id = |name: &str| if name == "ERROR" { Some(u16::MAX) } else { kinds.iter().position(|&k| k == name).map(|i| i as u16) };
    let stmts = ::bitlex::syntax::parse(grammar)?;
    let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let kind = |name: &str| {
        kind_id(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = OWN_IDS + own.len() as u16;
            if next >= 1 << rerail_tape_format::KIND_BITS {
                return None;
            }
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let field = |name: &str| fields.iter().position(|&f| f == name).map(|i| i as u16 + 1);
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field, terminators: rerail_tape_format::newlines::Terminators::Newlines };
    let compiled = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, known, bitlex_compile::AsRust::No)?;
    let program = rir::assemble(&compiled.text()).map_err(|e| format!("{e}\n{}", compiled.words))?;
    let mut native = native_ir::lower(&program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?;
    for b in &mut native.block_programs {
        b.pad = rerail_cpu_block::native_cpu_block::BACK as u16;
    }
    let own = own.borrow().len();
    Ok(((program, native, compiled.words, compiled.machine, own), compiled.proofs))
}

/// A directive as the tape gives it: `c_directive`'s kind and bytes (the file's).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub kind: &'static str,
    pub start: u32,
    pub end: u32,
    /// The conditional directive that opened the group it sits in (`c_cond_parent`): the chain's
    /// directive before it; an `#if`'s, `#elif`'s, `#else`'s or `#endif`'s, the group around
    /// its own chain's.
    pub parent: Option<u32>,
}

impl Compiled {
    /// Its tapes' format 16 table.
    pub fn lang(&self) -> &rerail_tape_format::f16::Lang {
        match &self.table {
            Table::Shared(lang) => lang,
            Table::Own(lang) => lang,
        }
    }

    /// The table's numbers by tier 2's keys, made once.
    fn numbers(&self) -> &native_cpu_stream::Numbers {
        self.numbers.get_or_init(|| self.parser.tier2.numbers(self.lang()))
    }

    /// `src`'s tape in format 16 as tier 2 writes it, or why the grammar refuses the file.
    pub fn tape(&self, src: &[u8]) -> Result<Tape16, String> {
        let w = self.parser.image16(src, TERMINATORS, self.lang(), self.numbers(), false)?;
        written(w).ok_or_else(|| "a 16z tape where format 16 was asked for".into())
    }

    /// `src`'s tape, or `None` where the grammar refuses the file.
    pub fn image(&self, src: &[u8]) -> Option<Tape16> {
        self.tape(src).ok()
    }

    /// `src`'s directives and their operands, read from its tape in place (`read`), or `None` where the grammar
    /// refuses it.
    pub fn read(&self, src: &[u8]) -> Option<Read> {
        read(&Nodes::of(&self.image(src)?, self.lang()).ok()?).ok()
    }

    /// `src`'s facts (`Facts::of` its tape's reading), or `None` where the grammar refuses it.
    pub fn facts(&self, src: &[u8]) -> Option<Facts> {
        self.read(src).map(|r| Facts::of(&r, src))
    }

    /// The same, by the interpreter: its tier 1 and its tier 2, the program's meaning.
    pub fn interpreter_read(&self, src: &[u8]) -> Option<Read> {
        read(&Nodes::of(&self.interpreter_image(src)?, self.lang()).ok()?).ok()
    }

    /// `src`'s tape by the interpreter: its tier 1 and its tier 2, the program's meaning (`stream::image16`).
    pub fn interpreter_image(&self, src: &[u8]) -> Option<Tape16> {
        let run = self.program.block_programs[0].run(src);
        let nodes = rerail_compiler::stream::run(&self.program, src, &run).ok()?;
        written(rerail_compiler::stream::image16(&nodes, &self.program.readings.extras, self.lang(), TERMINATORS, TAG, src).ok()?)
    }

    /// The same for each of `files`, tier 1 on Metal and tier 2 over its buffers; `None` for a
    /// file refused, and for all where the device cannot run the batch.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn metal_read(&self, files: &[&[u8]]) -> Result<Vec<Option<Read>>, rerail_compiler::Error> {
        Ok(self.metal_images(files)?.into_iter().map(|t| t.and_then(|t| read(&Nodes::of(&t, self.lang()).ok()?).ok())).collect())
    }

    /// Each of `files`' tapes, tier 1 on Metal and tier 2 over its buffers (`tier2_on::image16_on`); `None` for a
    /// file refused; the device's error where it cannot run the batch.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn metal_images(&self, files: &[&[u8]]) -> Result<Vec<Option<Tape16>>, rerail_compiler::Error> {
        use rerail_metal_programs::block_program::serial::{Segments, run_resident};
        let numbers = self.numbers();
        let batch: Vec<_> = files.iter().map(|f| rerail_compiler::block::BatchFile { bytes: f, program: 0 }).collect();
        match run_resident(&[&self.native], &batch, Segments { blocks: 29, warm: 1 }) {
            Ok((_, run)) => Ok((0..files.len()).map(|i| rerail_cpu_stream::tier2_on::image16_on(&self.parser, &run, i, TERMINATORS, self.lang(), numbers, false).ok().and_then(written)).collect()),
            Err(rerail_compiler::Error::BackendCapacity) if files.len() > 1 => {
                let (a, b) = files.split_at(files.len() / 2);
                let mut out = self.metal_images(a)?;
                out.extend(self.metal_images(b)?);
                Ok(out)
            }
            Err(e) => Err(e),
        }
    }
}

/// A file's directives, their operands and their conditionals' chains, as its tape gives them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub rows: Vec<Row>,
    pub operands: Vec<Operand>,
    /// Each conditional directive and the next of its chain (`c_cond_next`), in order.
    pub next: Vec<(u32, u32)>,
    /// The file's top level: each item's directive (a group's, its opener's), and whether code
    /// outside the directives is there.
    pub top: Vec<u32>,
    pub top_code: bool,
    /// An `#elif…`, `#else` or `#endif` no group is open for (the lean scan's unmatched), and an
    /// `#elif…` or `#else` after its chain's `#else`.
    pub unmatched: Vec<u32>,
    pub after_else: Vec<u32>,
    /// Each directive's word's end (its `#`'s where it has none): where an operand it lacks is.
    pub word_ends: Vec<u32>,
    /// The first directive's expression's leaves (at most 5: a guard's test), kind and bytes.
    pub first_expr: Vec<(&'static str, u32, u32)>,
}

/// A tape as text, a node a line: its depth's indent, its field (`name:`), its kind (by
/// `kinds`, its id where `kinds` has none) and its bytes. For tests and for reading a tape.
pub fn outline(v: &Nodes<'_>, kinds: &[&str], fields: &[&str], src: &[u8]) -> Result<String, String> {
    let mut out = String::new();
    let mut ends: Vec<usize> = Vec::new();
    for i in 0..v.len() {
        while ends.last().is_some_and(|&e| e <= i) {
            ends.pop();
        }
        let (k, f) = ((v.key(i) & 0x3ff) as usize, ((v.key(i) & 0xffff) >> rerail_tape_format::KIND_BITS) as usize);
        let kind = kinds.get(k).map_or_else(|| format!("#{k}"), |k| k.to_string());
        let field = f.checked_sub(1).and_then(|f| fields.get(f)).map_or(String::new(), |f| format!("{f}: "));
        let (s, e) = (v.start(i) as usize, v.end(i) as usize);
        let text = String::from_utf8_lossy(src.get(s..e.min(s + 40)).unwrap_or(b"")).replace('\n', "\\n");
        out += &format!("{}{field}{kind} {s}..{e} {text:?}\n", "  ".repeat(ends.len()));
        ends.push(i + v.size(i));
    }
    Ok(out)
}

/// A tape read in place, in one walk: each directive (a node of a directive's kind, at the
/// root or in a `group`), each field's node below it, and each group's chain (its own
/// directives) and the parent it gives the directives inside it.
pub fn read(v: &Nodes<'_>) -> Result<Read, String> {
    let kind = |k: usize| KINDS.get((v.key(k) & 0x3ff) as usize).copied().unwrap_or("");
    let field = |k: usize| FIELDS.get((((v.key(k) & 0xffff) >> rerail_tape_format::KIND_BITS) as usize).wrapping_sub(1)).copied();
    let mut out = Read { rows: Vec::new(), operands: Vec::new(), next: Vec::new(), top: Vec::new(), top_code: false, unmatched: Vec::new(), after_else: Vec::new(), word_ends: Vec::new(), first_expr: Vec::new() };
    // The open groups: each its end (a node index), its chain's last directive (`None` before
    // its opener) and whether its chain has had its `#else`.
    let mut groups: Vec<(usize, Option<u32>, bool)> = Vec::new();
    let mut i = 1;
    while i < v.len() {
        while groups.last().is_some_and(|g| g.0 <= i) {
            groups.pop();
        }
        let k = kind(i);
        if k == "group" {
            if groups.is_empty() {
                out.top.push(out.rows.len() as u32);
            }
            groups.push((i + v.size(i), None, false));
            i += 1;
            continue;
        }
        if k == "code" {
            out.top_code |= groups.is_empty();
            i += 1;
            continue;
        }
        let id = (v.key(i) & 0x3ff) as u16;
        if id >= LEAVES {
            return Err(format!("node {i}: {k} where a directive is"));
        }
        let d = out.rows.len() as u32;
        let chain = matches!(k, "elif" | "elifdef" | "elifndef" | "else" | "endif");
        if groups.is_empty() {
            out.top.push(d);
            if chain {
                out.unmatched.push(d);
            }
        }
        // A group's own directive: its opener (none yet), or a conditional at its level (a
        // nested `#if` opens a group of its own).
        let own = groups.last().is_some_and(|g| g.1.is_none() || chain);
        let parent = if own {
            let below = groups.len() - 1;
            if let Some(last) = groups[below].1 {
                out.next.push((last, d));
            }
            if groups[below].2 && k != "endif" {
                out.after_else.push(d);
            }
            groups[below].2 |= k == "else";
            groups[below].1 = Some(d);
            below.checked_sub(1).and_then(|b| groups[b].1)
        } else {
            groups.last().and_then(|g| g.1)
        };
        out.rows.push(Row { kind: k, start: v.start(i), end: v.end(i), parent });
        let end = i + v.size(i);
        // Its `#`'s leaf, then its word's (a line marker's number is its `rest`'s).
        let word = if i + 2 < end && field(i + 2).is_none() { i + 2 } else { i + 1 };
        out.word_ends.push(v.end(word));
        let mut last_param = false;
        for n in i + 1..end {
            let what = match field(n) {
                Some("name") => What::Name,
                Some("params") => What::Params,
                Some("param") => What::Param,
                // GNU's `name...`: the parameter's leaf right before it (a `,` is a leaf of none).
                Some("variadic") if last_param => What::NamedVariadic,
                Some("variadic") => What::Variadic,
                Some("body") => What::Body,
                Some("spelled") => match kind(n) {
                    "string" => What::Quote,
                    "header" => What::Angle,
                    _ => What::Macro,
                },
                Some("expr") => What::Expr,
                Some("text") => What::Text,
                Some("first") => What::First,
                Some("second") => What::Second,
                Some("n") => What::Number,
                Some("file") => What::File,
                Some("rest") => What::Rest,
                _ => {
                    last_param = false;
                    continue;
                }
            };
            last_param = what == What::Param;
            out.operands.push(Operand { i: d, what, start: v.start(n), end: v.end(n) });
            if d == 0 && what == What::Expr && v.size(n) <= 6 {
                out.first_expr = (n + 1..n + v.size(n)).map(|l| (kind(l), v.start(l), v.end(l))).collect();
            }
        }
        i = end;
    }
    Ok(out)
}


/// An operand of directive `i` (its index among the file's directives): what it is and its
/// bytes in the file, as `c_define`, `c_include` and the rest of 0327's mode (a) relations take
/// them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Operand {
    pub i: u32,
    pub what: What,
    pub start: u32,
    pub end: u32,
}

/// What an operand is. A function-like macro's parameter list is `Params` (its bytes, the
/// parentheses too); its variadic parameter is `Variadic` (`...`) or `NamedVariadic` (GNU's
/// `name...`, the last `Param`). An include's header is `Quote`, `Angle` or `Macro` by its form.
/// `Number` is a line's number where its token is one (`value`'s bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum What {
    Name,
    Params,
    Param,
    Variadic,
    NamedVariadic,
    Body,
    Quote,
    Angle,
    Macro,
    Expr,
    Text,
    First,
    Second,
    Number,
    File,
    Rest,
}


/// A tape as text: a node a line, indented by depth, with its span.
pub fn image_text(v: &Nodes<'_>) -> Result<String, String> {
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

mod facts;
pub use facts::Facts;
pub mod expand;
pub mod tokens;
pub mod pp;
pub mod eval;

/// `src`'s mode (a) facts: from the compiled grammar's tape where it reads the file, else (a
/// construct the grammar does not read, or a grammar that does not compile) the lean scan's.
pub fn facts(src: &[u8], opts: rerail_c_pp::Options) -> Facts {
    // The grammar reads raw strings: a configuration without them (C, not GNU) has the lean scan
    // read a file that may have one.
    let raw_free = opts.raw_strings || rerail_simd8::micro::find_pair(src, 0, b'R', b'"').is_none();
    raw_free.then(compiled).and_then(Result::ok).and_then(|c| c.facts(src)).unwrap_or_else(|| Facts::lean(src, opts))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod answers_tests;
