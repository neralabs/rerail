//! The `.rr` parser: the Bitlex grammar of `.rr` (the last step of `rr_syntax/bitlex_ladder/`), compiled
//! once per process through the Bitlex compiler to tier 1 and tier 2 on Cranelift, and shared by
//! every thread.
//!
//! A crate of its own, on the compiler crate, the Metal crate and the engine's `.rr` syntax
//! (`rerail_rr_syntax`), so the parser and its measurements rebuild without the engine.
//!
//! `tape` gives the engine's own tape (`rerail_rr_syntax::tape`): the same kinds, fields, spans and
//! parents. The grammar's kinds and fields take the engine's ids by name, so no table maps them.
//!
//! A file the compiled parser refuses goes to the engine's parser (`rerail_rr_syntax::tape`). On the
//! corpus that is a file the engine refuses too, or a pathological one the grammar leaves to it
//! on purpose: a run of four backticks or more, braces nested past 127.

use rerail_compiler::bitlex_masks as bitlex_compile;
pub use rerail_compiler::native_cpu_stream::{Columns, NO_PARENT, NodeRow};
use rerail_compiler::{native_cpu_stream, native_ir, rir_asm as rir};
use rerail_rr_syntax as rr;
use std::cell::RefCell;
use std::collections::HashMap;

/// The grammar: the ladder's last step.
const GRAMMAR: &str = include_str!("../../rr_syntax/bitlex_ladder/21-named-only.bitlex");

/// Sample documents, for the profile tier 2 is cut by when the grammar compiles
/// (docs/parser-generators/28-tier-2-in-clusters.md): 12 of the repository's `.rr`
/// files, spread over its folders. A profile of about ten files gives a cut as good as the
/// whole corpus's.
const SAMPLES: [&str; 12] = [
    include_str!("../samples/data-layer-graph-lazy-sql-plan.rr.sample"),
    include_str!("../samples/eslint-no-sequences.rr.sample"),
    include_str!("../samples/node.rr.sample"),
    include_str!("../samples/phpcs-generic-php-forbidden-functions.rr.sample"),
    include_str!("../samples/plan-scope-side-effects.rr.sample"),
    include_str!("../samples/react-hooks-rules-of-hooks.rr.sample"),
    include_str!("../samples/ruff-b029-except-with-empty-tuple.rr.sample"),
    include_str!("../samples/ruff-fbt001-boolean-type-hint-positional-argument.rr.sample"),
    include_str!("../samples/ruff-pth115-os-readlink.rr.sample"),
    include_str!("../samples/ruff-ruf043-pytest-raises-ambiguous-pattern.rr.sample"),
    include_str!("../samples/swallowed-exceptions.rr.sample"),
    include_str!("../samples/typescript-eslint-no-dynamic-delete.rr.sample"),
];

/// The cap of tier 2's clusters, in words (doc 28: 2,500 runs fastest on `.rr`).
const CLUSTER_CAP: usize = 2500;

/// Ids above the engine's, for the grammar's own names (supertypes, marks) that never reach a
/// tape.
const OWN_IDS: u16 = 0x4000;

/// The compiled parser.
pub struct Compiled {
    pub parser: native_cpu_stream::Parser,
    /// Tier 1's cost by grammar statement, for the profile.
    pub costs: Vec<bitlex_compile::Cost>,
    /// Each tier 1 operation's statement (`bitlex_compile::Compiled::origin`), for the profile.
    pub origin: Vec<String>,
    /// Tier 1's named values (`bitlex_compile::Compiled::values`), for the analyses (`token_local`).
    pub values: Vec<bitlex_compile::Named>,
    /// Tier 1's block program, as assembled.
    pub block: rerail_compiler::block::BlockProgram,
    /// Each output's label (`bitlex_compile::Compiled::labels`).
    pub labels: Vec<(String, u16)>,
    /// Tier 1's program as lowered, for its other executors (Metal).
    pub native: rerail_compiler::block_native::BlockNative,
    /// Tier 1's block program as text, for the profile's operation mix.
    pub rows: String,
    /// Tier 2's words and its state machine as text, for the profile.
    pub words: String,
    pub machine: String,
    /// The program as assembled: the interpreter's input, for the analyses.
    pub program: rerail_compiler::program::Words,
}

// SAFETY: nothing changes a compiled parser once it is made. Its JIT module holds finished code
// and is only kept alive; `parse` takes `&self`, reads the code and its tables, and writes only
// buffers it allocates for the call. So threads may share one.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

/// The one compiled parser of the process, made at its first use.
static PARSER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled parser, compiling it the first time.
pub fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(compile).as_ref().map_err(String::as_str)
}

fn compile() -> Result<Compiled, String> {
    // Tier 2 a function a grammar rule, from the grammar (`Shape::Rules`); `RR_SHAPE=machine`:
    // one state machine.
    let shape = if std::env::var("RR_SHAPE").as_deref() == Ok("machine") { bitlex_compile::Shape::Machine } else { bitlex_compile::Shape::Rules };
    compile_with(shape)
}

/// The grammar compiled with tier 2 in `shape`.
pub fn compile_with(shape: bitlex_compile::Shape) -> Result<Compiled, String> {
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
    let compiled = bitlex_compile::compile_shaped(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), shape)?;
    let program = rir::assemble(&compiled.text())?;
    let native = native_ir::lower(&program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?;
    // Tier 2 a function a grammar rule (`by_rules`): a call of the grammar is a native call,
    // its return a native return; the optimizer on. Its entries and arms weighted by the
    // samples' traffic. `RR_CUT=traffic`: the clusters cut by traffic instead, as before.
    let samples: Vec<&[u8]> = SAMPLES.iter().map(|s| s.as_bytes()).collect();
    let traffic = rerail_compiler::native_cpu_clusters::traffic(&program, &samples);
    let cut = if std::env::var("RR_CUT").as_deref() == Ok("traffic") {
        rerail_compiler::native_cpu_clusters::by_traffic(&native, &traffic, CLUSTER_CAP)
    } else {
        rerail_compiler::native_cpu_clusters::Cut { pick_traffic: traffic.clone(), near: true, unsigned: true, speed: true, ..rerail_compiler::native_cpu_clusters::by_rules(&native) }
    };
    let parser = native_cpu_stream::Parser::with_cut(&native, &cut).map_err(|e| format!("Cranelift: {e:?}"))?;
    Ok(Compiled { parser, native: native.block_programs[0].clone(), costs: compiled.costs.clone(), origin: compiled.origin.clone(), values: compiled.values.clone(), block: program.block_programs[0].clone(), labels: compiled.labels.clone(), rows: compiled.rows.clone(), words: compiled.words.clone(), machine: compiled.machine.clone(), program })
}

/// `src`'s node table, as tier 2 writes it: from the compiled grammar, or, for a file the grammar
/// refuses, from the engine's parser (`logic::rr::tape`) in the same columns.
pub fn tape(src: &str) -> Columns {
    compiled_tape(src).unwrap_or_else(|| engine_columns(&rr::tape(src)))
}

/// The compiled grammar's node table, or `None` where it refuses the file (or did not compile).
/// Every column is the compiled code's: kinds, fields, spans and sizes from tier 2's words,
/// parents and extra flags from its final pass.
pub fn compiled_tape(src: &str) -> Option<Columns> {
    compiled().ok()?.parser.columns(src.as_bytes()).ok()
}

/// The node tables of `files`, in order, as `tape` gives each: tier 1 over the whole batch on
/// the device (Metal's forward carry), then tier 2 over each file reading tier 1's buffers where
/// the device left them; without a device, `tape` file by file.
pub fn tapes(files: &[&str]) -> Vec<Columns> {
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
const SEGMENTS: rerail_metal::block_program::serial::Segments = rerail_metal::block_program::serial::Segments { blocks: 29, warm: 1 };

/// `tapes` on Metal: halves while the device refuses the batch for capacity; `false` when it
/// cannot run it at all.
#[cfg(target_os = "macos")]
fn metal_tapes(c: &Compiled, files: &[&str], out: &mut Vec<Columns>) -> bool {
    use rerail_metal::block_program::serial::run_resident;
    let mut batch = Vec::with_capacity(files.len());
    for file in files {
        batch.push(rerail_compiler::block::BatchFile { bytes: file.as_bytes(), program: 0 });
    }
    match run_resident(&[&c.native], &batch, SEGMENTS) {
        Ok((_, run)) => {
            let mut i = 0;
            for file in files {
                out.push(rerail_metal::tier2_on::columns_on(&c.parser, &run, i).unwrap_or_else(|_| engine_columns(&rr::tape(file))));
                i += 1;
            }
            true
        }
        Err(rerail_compiler::Error::BackendCapacity) if files.len() > 1 => {
            let (a, b) = files.split_at(files.len() / 2);
            metal_tapes(c, a, out) && metal_tapes(c, b, out)
        }
        Err(_) => false,
    }
}

/// The fallback: the engine's parser's nodes, in the compiled parser's rows.
pub fn engine_columns(nodes: &[rr::tape::Node]) -> Columns {
    let mut rows: Vec<NodeRow> = nodes
        .iter()
        .map(|n| NodeRow { kind: n.kind, field: n.field, start: n.start, end: n.end, size: 1, parent: n.parent.unwrap_or(NO_PARENT), extra: u8::from(n.extra), pad: [0; 3] })
        .collect();
    // In preorder a node comes after its parent: sizes from the last node back.
    for (i, n) in nodes.iter().enumerate().rev() {
        if let Some(p) = n.parent {
            rows[p as usize].size += rows[i].size;
        }
    }
    Columns { rows }
}

/// Why the grammar did not compile, if it did not: a test's check.
#[cfg(test)]
pub fn compile_error() -> Option<String> {
    compiled().err().map(str::to_owned)
}

#[cfg(test)]
mod gating_tests;
#[cfg(test)]
mod token_local_tests;

#[cfg(test)]
pub mod tests {
    /// The compiled grammar's tape is the engine's, node for node (extras and parents too), on
    /// documents it parses; the ones it leaves to the engine come back from there.
    /// Tier 2's grammar calls are the machine's (`by_rules`): nested past `DEPTH`, a file is
    /// refused (to the fallback), not run off the stack. Nested within it, it parses.
    #[test]
    fn deep_nesting_is_refused_not_overflowed() {
        let nested = |n: usize| format!(".decl r(x: number)\nr(x) :- x = {}1{}.\n", "(".repeat(n), ")".repeat(n));
        let c = crate::compiled().expect("compiles");
        let shallow = nested(100);
        assert_eq!(c.parser.columns(shallow.as_bytes()).ok(), Some(crate::engine_columns(&crate::rr::tape(&shallow))), "100 deep parses as the engine's");
        let deep = nested(3_000);
        assert!(c.parser.columns(deep.as_bytes()).is_err(), "3,000 deep: refused");
        // (The engine's parser, recursive too, runs off a test thread's stack at that depth.)
    }

    /// Tier 2 is a function a grammar rule, from the grammar: the program holds its rules
    /// (`Words::rules`, a `rir` row each), the verifier checked that only calls cross them, and
    /// each rule is one function (`by_rules` reads them; none is found). A call takes no token:
    /// its arm goes to the callee's switch. The state machine is still there
    /// (`Shape::Machine`): no rules. Both write the engine's tapes.
    #[test]
    fn tier2_is_a_function_a_grammar_rule() {
        use rerail_compiler::{native_cpu_clusters, native_ir, stream};
        let c = crate::compiled().expect("compiles");
        let rules = &c.program.rules;
        assert!(rules.len() > 20, "a rule a level: {}", rules.len());
        assert!(c.words.contains("  rule 0 @0000\n"), "the rules are rows of the program's text");
        assert!(c.machine.contains(" call"), "calls take no token");
        // A ratchet, as the machine's (`the_state_machine_shares_its_rules`): lower it as the
        // compiler improves. Keywords are the switches' targets (`ByteSwitch::words`): 13,119
        // words before, when each state tested each keyword with a `SPAN`, a `WORD` and jumps.
        // A third are still jumps (`GOTO`).
        assert!(c.program.switches.iter().any(|s| !s.words.is_empty()), "the switches name keywords");
        // Its words in order, by the verifier: out of order, refused.
        let mut bad = c.program.clone();
        let named = bad.switches.iter_mut().find(|s| s.words.len() > 1).expect("a switch naming two words");
        named.words.swap(0, 1);
        assert!(stream::validate(&bad).is_err(), "a switch's words out of order");
        let states = c.machine.lines().filter(|l| l.starts_with("state ")).count();
        let arms = c.machine.lines().filter(|l| l.starts_with("arm ")).count();
        assert!(states <= 371 && arms <= 3068 && c.program.words.len() <= 8860, "{states} states, {arms} arms, {} words", c.program.words.len());
        let native = native_ir::lower(&c.program, 0, 0).expect("lowers");
        let cut = native_cpu_clusters::by_rules(&native);
        assert_eq!((cut.count as usize, cut.calls), (rules.len(), true), "a function a rule");
        assert_eq!(c.parser.tier2.clusters, rules.len(), "as compiled");
        // The calls open counted by the machine's stack, as the lean parser's: no `RETURN` runs
        // with no call open, and no `ACCEPT` within one, so neither is checked.
        assert!(rerail_compiler::native_cpu_stream::calls_on_stack(&native), "the calls open on the stack");
        // A jump from one rule into a later one, not a call's: refused (into an earlier rule's
        // code, which rules share, it is a call of that rule's function).
        let mut bad = c.program.clone();
        let (from, into) = (usize::from(rules[1]), rules[2]);
        let goto = (0..bad.words.len()).find(|&pc| pc >= from && rerail_compiler::program::split(bad.words[pc]).0 == rerail_compiler::program::GOTO && rerail_compiler::program::split(bad.words[pc - 1]).0 != rerail_compiler::program::PUSH).expect("a rule's own jump");
        bad.words[goto] = rerail_compiler::program::word(rerail_compiler::program::GOTO, 0, into);
        assert!(stream::validate_rules(&bad).is_err(), "a jump across rules");
        // The machine: no rules. Each shape writes the engine's tape on the samples.
        let m = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        assert!(m.program.rules.is_empty());
        for (i, s) in crate::SAMPLES.iter().enumerate() {
            let engine = Some(crate::engine_columns(&crate::rr::tape(s)));
            assert_eq!(c.parser.columns(s.as_bytes()).ok(), engine, "sample {i}, as rules");
            assert_eq!(m.parser.columns(s.as_bytes()).ok(), engine, "sample {i}, as one machine");
        }
    }

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

    /// Tier 2's state machine is built once per rule, not once per place a rule is used: rules
    /// whose calls save states are calls (`grammar::shared`). With every rule inlined into each
    /// place, `.rr` had 858 states and 8,370 arms; shared, 438 and 4,014. A ratchet: lower it as
    /// the compiler improves. Named-only tapes (step 21) raised it from 234 and 2,225 (to 266 and 2,590): a group's
    /// body and an aggregate word's arguments are rules of their own, which drop their `,`.
    /// Of the state machine (`Shape::Machine`); the product's shape, a function a rule, has its
    /// own (`tier2_is_a_function_a_grammar_rule`).
    #[test]
    fn the_state_machine_shares_its_rules() {
        let c = super::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles");
        let states = c.machine.lines().filter(|l| l.starts_with("state ")).count();
        let arms = c.machine.lines().filter(|l| l.starts_with("arm ")).count();
        assert!(states <= 266, "{states} states");
        assert!(arms <= 2590, "{arms} arms");
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
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
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
            ("lib.rs", "pub fn tape(src: &str)"),
            ("lib.rs", "pub fn compiled_tape("),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn columns(&self, bytes: &[u8])"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn columns(mut self)"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn tape(&self, batch"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn tape(&self, n: usize)"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn words(&self, batch"),
            ("../../compiler/src/native_cpu_stream.rs", "fn stream<'b>(&self"),
            ("../../compiler/src/native_cpu_stream.rs", "pub unsafe fn call(&self, bytes"),
            ("../../metal/src/tier2_on.rs", "pub fn columns_on(parser: &Parser"),
            ("../../compiler/src/native_cpu_stream.rs", "unsafe fn call_once(&self"),
            ("../../compiler/src/native_cpu_stream.rs", "fn full_tape(&self"),
            ("../../compiler/src/native_cpu_stream.rs", "fn sized(&self"),
            ("lib.rs", "pub fn tapes("),
            ("lib.rs", "fn metal_tapes("),
            ("../../compiler/src/native_cpu_block.rs", "pub fn new(program: &CompiledBlock, files"),
            ("../../compiler/src/native_cpu_block.rs", "pub fn call(&mut self, program: &CompiledBlock)"),
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
        use rerail_compiler::block;
        use rerail_metal::block_program::serial::{Segments, run_serial_split};
        let c = super::compiled().expect("compiles");
        let files = corpus();
        let refs: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        let want = c.parser.tier1.run_files(&refs);
        let batch: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
        let native = &c.native;
        let got = run_serial_split(&[native], &batch, Segments { blocks: 29, warm: 1 }).expect("Metal runs tier 1");
        let mut differ = 0;
        for (k, (g, w)) in got.run.files.iter().zip(&want).enumerate() {
            if g.outputs != w.outputs || g.positions != w.positions || g.totals != w.totals || g.codes != w.codes {
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

    /// The compiled parser's tier 2 is in clusters, cut by the samples' profile, and each sample
    /// parses (none is refused).
    #[test]
    fn tier2_is_in_clusters_by_the_samples() {
        let c = crate::compiled().expect("compiles");
        assert!(c.parser.tier2.clusters > 1, "{} clusters", c.parser.tier2.clusters);
        for (i, s) in crate::SAMPLES.iter().enumerate() {
            assert!(crate::compiled_tape(s).is_some(), "sample {i} refused");
        }
    }

    /// Each corpus file the compiled grammar takes, as rules and as one machine: the engine's
    /// tape. Prints how many each takes.
    #[test]
    #[ignore = "the corpus: run by name"]
    fn corpus_tapes_are_the_engines() {
        let files = corpus();
        let rules = crate::compiled().expect("compiles");
        let machine = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        let mut taken = [0usize; 2];
        let mut differ = Vec::new();
        for (i, f) in files.iter().enumerate() {
            let engine = crate::engine_columns(&crate::rr::tape(f));
            for (k, c) in [rules, &machine].into_iter().enumerate() {
                if let Ok(t) = c.parser.columns(f.as_bytes()) {
                    taken[k] += 1;
                    if t != engine {
                        differ.push((i, k));
                    }
                }
            }
        }
        eprintln!("{} files: rules take {}, one machine {}", files.len(), taken[0], taken[1]);
        assert_eq!(differ, Vec::new(), "(file, shape) whose tape is not the engine's");
        assert_eq!(taken[0], taken[1], "the shapes take the same files");
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

    /// Tier 2's states cut into clusters, for a function a cluster: each state (a `TOKEN` word)
    /// and the words it reaches before the next `TOKEN` (its code), and how often the corpus
    /// goes from each state to each (the interpreter's run). Clusters are merged greedily, the
    /// two with the most traffic between them first, while the merged code stays under a cap
    /// (in words; tier 2 is about 12 instructions a word). For each cap: the clusters, the
    /// largest, and the traffic that crosses clusters (a call or a return a crossing).
    #[test]
    #[ignore = "an analysis: run by hand"]
    fn rr_state_clusters() {
        use rerail_compiler::program::*;
        use std::collections::{BTreeSet, HashMap};
        let t = thread_cpu();
        let c = super::compiled().expect("compiles");
        eprintln!("the grammar compiled in {:.2?} CPU", thread_cpu() - t);
        let program = &c.program;
        // Cranelift's share: tier 1 and tier 2 made again from the lowered program.
        let native = rerail_compiler::native_ir::lower(program, 0, 0).expect("lowers");
        let t = thread_cpu();
        let _t1 = rerail_compiler::native_cpu_block::compile(&native.block_programs[0]).expect("tier 1");
        let t1 = thread_cpu() - t;
        let t = thread_cpu();
        let _ = cranelift_codegen::timing::take_current();
        let _t2 = rerail_compiler::native_cpu_stream::compile(&native).expect("tier 2");
        eprintln!("Cranelift: tier 1 {t1:.2?} CPU, tier 2 {:.2?} CPU", thread_cpu() - t);
        // Where tier 2's compile goes, by Cranelift's passes (the register allocator's share
        // is what grows faster than the function).
        eprintln!("tier 2's passes:\n{}", cranelift_codegen::timing::take_current());
        let words = &program.words;
        let refuse = rerail_compiler::stream_ir::REFUSE;
        let flag = rerail_compiler::program::FLAG;
        // Where a `RETURN` goes: the places after each `PUSH`'s target, by its mode.
        let pushes: Vec<usize> = words.iter().filter_map(|&w| (split(w).0 == PUSH).then(|| usize::from(split(w).2))).collect();
        let next = |pc: usize| -> Vec<usize> {
            let (op, mode, x) = split(words[pc]);
            match op {
                STOP | ACCEPT => vec![],
                RETURN => pushes.iter().map(|&p| p + usize::from(mode)).collect(),
                GOTO => vec![usize::from(x)],
                SWITCH => {
                    let sw = &program.switches[usize::from(x)];
                    sw.all_targets().filter(|&t| t != refuse).map(usize::from).collect()
                }
                WORD_IS if mode == 1 => vec![pc + 1, pc + 2],
                o if o == flag && mode == 2 => vec![pc + 1, pc + 2],
                MARK | ROW => vec![pc + 1, pc + 2],
                _ => vec![pc + 1],
            }
        };
        let states: Vec<usize> = (0..words.len()).filter(|&pc| split(words[pc]).0 == TOKEN).collect();
        let index: HashMap<usize, usize> = states.iter().enumerate().map(|(i, &pc)| (pc, i)).collect();
        // A state's code: the words from its `TOKEN` to the next `TOKEN`s (a `RETURN`'s places
        // are its callers', not followed).
        let code: Vec<BTreeSet<usize>> = states
            .iter()
            .map(|&s| {
                let (mut seen, mut work) = (BTreeSet::new(), vec![s]);
                while let Some(pc) = work.pop() {
                    if pc >= words.len() || !seen.insert(pc) {
                        continue;
                    }
                    if split(words[pc]).0 == RETURN {
                        continue;
                    }
                    for n in next(pc) {
                        if n < words.len() && (split(words[n]).0 != TOKEN || n == s) {
                            work.push(n);
                        }
                    }
                }
                seen
            })
            .collect();
        // The traffic: each token's state after the one before.
        let block = &program.block_programs[0];
        let mut edges: HashMap<(usize, usize), u64> = HashMap::new();
        let (mut tokens, mut ran) = (0u64, 0u64);
        for src in corpus() {
            let run = block.run(src.as_bytes());
            let mut last: Option<usize> = None;
            let _ = rerail_compiler::stream::run_seen(program, src.as_bytes(), &run, &mut |pc| {
                ran += 1;
                if let Some(&i) = index.get(&pc) {
                    tokens += 1;
                    if let Some(l) = last {
                        *edges.entry((l, i)).or_default() += 1;
                    }
                    last = Some(i);
                }
            });
        }
        let total_words: usize = code.iter().map(BTreeSet::len).sum();
        let distinct: BTreeSet<usize> = code.iter().flatten().copied().collect();
        eprintln!("{} words, {} states; a state's code {} words in all ({} distinct: words shared by states); {tokens} tokens, {ran} words run", words.len(), states.len(), total_words, distinct.len());
        let mut sizes: Vec<usize> = code.iter().map(BTreeSet::len).collect();
        sizes.sort_unstable();
        eprintln!("  a state's code: median {} words, 90% {}, largest {}", sizes[sizes.len() / 2], sizes[sizes.len() * 9 / 10], sizes[sizes.len() - 1]);
        let mut pairs: HashMap<(usize, usize), u64> = HashMap::new();
        for (&(a, b), &n) in &edges {
            if a != b {
                *pairs.entry((a.min(b), a.max(b))).or_default() += n;
            }
        }
        let crossing_all: u64 = pairs.values().sum();
        // The static graph: a state to each state its code goes on to, weighted by the edges.
        let mut fixed: HashMap<(usize, usize), u64> = HashMap::new();
        for (i, words_of) in code.iter().enumerate() {
            for &pc in words_of {
                for n in next(pc) {
                    if let Some(&j) = index.get(&n) {
                        if j != i {
                            *fixed.entry((i.min(j), i.max(j))).or_default() += 1;
                        }
                    }
                }
            }
        }
        eprintln!("  transitions between two different states: {crossing_all} ({:.2} a token)", crossing_all as f64 / tokens as f64);
        // A walk taking each of a state's ways on equally often: its flow along each edge, from
        // the walk's stationary distribution (power iteration), scaled to integers.
        let n = states.len();
        let mut out: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
        for (i, words_of) in code.iter().enumerate() {
            for &pc in words_of {
                for x in next(pc) {
                    if let Some(&j) = index.get(&x) {
                        *out[i].entry(j).or_default() += 1.0;
                    }
                }
            }
        }
        let mut pi = vec![1.0 / n as f64; n];
        for _ in 0..500 {
            let mut nx = vec![0.15 / n as f64; n];
            for i in 0..n {
                let sum: f64 = out[i].values().sum();
                if sum == 0.0 {
                    for v in nx.iter_mut() {
                        *v += 0.85 * pi[i] / n as f64;
                    }
                    continue;
                }
                for (&j, &w) in &out[i] {
                    nx[j] += 0.85 * pi[i] * w / sum;
                }
            }
            pi = nx;
        }
        let mut walk: HashMap<(usize, usize), u64> = HashMap::new();
        for i in 0..n {
            let sum: f64 = out[i].values().sum();
            for (&j, &w) in &out[i] {
                if i != j {
                    *walk.entry((i.min(j), i.max(j))).or_default() += (pi[i] * w / sum * 1e9) as u64;
                }
            }
        }
        for (by, weights) in [("the corpus's traffic", &pairs), ("the static graph", &fixed), ("a walk's flow (static)", &walk)] {
        eprintln!(" clusters by {by}; crossings counted on the corpus");
        for cap in [300usize, 600, 1200, 2500] {
            let mut of: Vec<usize> = (0..states.len()).collect();
            let mut members: Vec<BTreeSet<usize>> = code.clone();
            let mut order: Vec<((usize, usize), u64)> = weights.iter().map(|(&k, &n)| (k, n)).collect();
            order.sort_by(|a, b| b.1.cmp(&a.1));
            // Weights between clusters, merged as clusters merge: repeat until nothing merges.
            loop {
                let mut between: HashMap<(usize, usize), u64> = HashMap::new();
                for &((a, b), n) in &order {
                    let (x, y) = (of[a], of[b]);
                    if x != y {
                        *between.entry((x.min(y), x.max(y))).or_default() += n;
                    }
                }
                let mut cand: Vec<((usize, usize), u64)> = between.into_iter().collect();
                cand.sort_by(|a, b| b.1.cmp(&a.1));
                let mut merged = false;
                for ((x, y), _) in cand {
                    if members[x].len() + members[y].len() > cap && members[x].union(&members[y]).count() > cap {
                        continue;
                    }
                    let moved = std::mem::take(&mut members[y]);
                    members[x].extend(moved);
                    for o in of.iter_mut() {
                        if *o == y {
                            *o = x;
                        }
                    }
                    merged = true;
                    break;
                }
                if !merged {
                    break;
                }
            }
            let clusters: BTreeSet<usize> = of.iter().copied().collect();
            let crossing: u64 = pairs.iter().filter(|(k, _)| of[k.0] != of[k.1]).map(|(_, n)| *n).sum();
            let largest = clusters.iter().map(|&k| members[k].len()).max().unwrap_or(0);
            let words_in_all: usize = clusters.iter().map(|&k| members[k].len()).sum();
            eprintln!("  cap {cap:>5} words: {:>4} clusters, largest {largest:>5} words, {words_in_all:>6} words in all; crossings {crossing:>8} ({:.3} a token)", clusters.len(), crossing as f64 / tokens as f64);
        }
        }
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

    /// Where a Metal run's wall time goes: two runs in a row over the corpus's first files.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "a probe: run by hand"]
    fn rr_metal_probe() {
        use rerail_metal::block_program::serial::run_resident;
        let w = std::time::Instant::now();
        let c = super::compiled().expect("compiles");
        eprintln!("compiled in {:.2?} wall", w.elapsed());
        let files = corpus();
        for n in [100usize] {
            let batch: Vec<_> = files.iter().take(n).map(|f| rerail_compiler::block::BatchFile { bytes: f.as_bytes(), program: 0 }).collect();
            for k in 0..4 {
                let (t, w) = (thread_cpu(), std::time::Instant::now());
                let r = run_resident(&[&c.native], &batch, super::SEGMENTS);
                let ok = match &r {
                    Ok((run, _)) => format!("GPU {:.2} ms, {} launches, {} segments, {} reruns, {} rounds", run.run.seconds * 1e3, run.run.launches, run.segments, run.reruns, run.rounds),
                    Err(e) => format!("{e:?}"),
                };
                eprintln!("{n} files, run {k}: {:.2?} wall, {:.2?} CPU; {ok}", w.elapsed(), thread_cpu() - t);
            }
        }
    }

    /// Tier 1's passes: each forward pass's instructions, scans, groups and `prev`s, and the
    /// cone of `RR_CONE` (an instruction of the pass `RR_PASS`).
    #[test]
    #[ignore = "a probe: run by hand"]
    fn rr_tier1_shape() {
        use rerail_compiler::block_native::{BlockInst, Pass};
        let c = super::compiled().expect("compiles");
        let passes = c.native.passes();
        let want: Option<(usize, usize)> = std::env::var("RR_PASS").ok().and_then(|p| Some((p.parse().ok()?, std::env::var("RR_CONE").ok()?.parse().ok()?)));
        for (k, pass) in passes.passes.iter().enumerate() {
            match pass {
                Pass::Back(fills) => eprintln!("pass {k}: back, {} fills", fills.len()),
                Pass::Forward { body, stores } => {
                    let b = body.planes_form();
                    let count = |f: &dyn Fn(&BlockInst) -> bool| b.insts.iter().filter(|i| f(i)).count();
                    eprintln!(
                        "pass {k}: forward, {} insts, {} toggles, {} escapes, {} fills, {} prevs, {} groups ({:?} members), {} outputs, {} stores",
                        b.insts.len(),
                        count(&|i| matches!(i, BlockInst::Toggle(_))),
                        count(&|i| matches!(i, BlockInst::Escaped(_))),
                        count(&|i| matches!(i, BlockInst::Fill(..))),
                        count(&|i| matches!(i, BlockInst::Prev(..))),
                        b.groups.len(),
                        b.groups.iter().map(|g| (g.members.len(), g.carries.len())).collect::<Vec<_>>(),
                        b.outputs.len(),
                        stores.len()
                    );
                    if let Some((p, at)) = want.filter(|w| w.0 == k) {
                        let mut inside = vec![false; b.insts.len()];
                        inside[at] = true;
                        for x in (0..=at).rev() {
                            if inside[x] {
                                for o in b.insts[x].operands() {
                                    inside[o] = true;
                                }
                            }
                        }
                        let _ = p;
                        for (x, i) in b.insts.iter().enumerate().filter(|(x, _)| inside[*x]) {
                            eprintln!("  %{x} = {i:?}");
                        }
                    }
                }
            }
        }
    }

    /// Tier 1 of `.rr` on Metal, its kernels made first: `prewarm` over a sample (`RR_SAMPLE`)
    /// runs the program, learns, and compiles the learned program's kernels, as a
    /// process would off the parse's path. The timed runs then must compile nothing
    /// (`compiles`). Each run: the corpus (`RR_REPEAT` times) as batches under the device's
    /// budget, halved where refused; its GPU seconds by kind (the first pass, the runs again,
    /// the positions), its launches and rounds, and the host's CPU and wall.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_metal_bench() {
        use rerail_metal::block_program::serial::{compiles, prewarm, run_resident_digested, SerialRun};
        let c = super::compiled().expect("compiles");
        let n: usize = std::env::var("RR_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let files: Vec<String> = (0..n).flat_map(|_| corpus()).collect();
        let batch: Vec<_> = files.iter().map(|f| rerail_compiler::block::BatchFile { bytes: f.as_bytes(), program: 0 }).collect();
        let digests: Vec<[u8; 32]> = files.iter().map(|f| <sha2::Sha256 as sha2::Digest>::digest(f.as_bytes()).into()).collect();
        let mb = files.iter().map(String::len).sum::<usize>() as f64 / 1e6;
        let w = std::time::Instant::now();
        // The sample: every `RR_SAMPLE`th file (every file by default: what a run learns
        // depends on the files, and a sample of a quarter left 12 kernels to the timed runs).
        let every: usize = std::env::var("RR_SAMPLE").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let sample: Vec<_> = batch.iter().step_by(every).cloned().collect();
        prewarm(&[&c.native], &sample, super::SEGMENTS).expect("prewarm");
        eprintln!("prewarm: {:.2?} wall, {} sources and kernels made", w.elapsed(), compiles());
        // One run of the batch, halved where the device refuses it: summed.
        fn parts(c: &super::Compiled, b: &[rerail_compiler::block::BatchFile<'_>], d: &[[u8; 32]], sum: &mut [f64; 6]) {
            match run_resident_digested(&[&c.native], b, Some(d), super::SEGMENTS) {
                Ok((r, _)) => {
                    let r: SerialRun = r;
                    for (x, v) in sum.iter_mut().zip([r.first_seconds, r.rerun_seconds, r.sink_seconds, r.run.launches as f64, r.rounds as f64, 1.0]) {
                        *x += v;
                    }
                }
                Err(rerail_metal::Error::BackendCapacity) if b.len() > 1 => {
                    let h = b.len() / 2;
                    parts(c, &b[..h], &d[..h], sum);
                    parts(c, &b[h..], &d[h..], sum);
                }
                Err(e) => panic!("{e:?}"),
            }
        }
        let before = compiles();
        for k in 0..4 {
            let made = rerail_metal::pool::made();
            let (t, w) = (thread_cpu(), std::time::Instant::now());
            let mut sum = [0.0; 6];
            parts(c, &batch, &digests, &mut sum);
            let (cpu, wall) = (thread_cpu() - t, w.elapsed());
            let now = rerail_metal::pool::made();
            eprintln!("run {k}: {} blocks made, {} MB", now.0 - made.0, (now.1 - made.1) >> 20);
            let gpu = sum[0] + sum[1] + sum[2];
            eprintln!(
                "run {k}: {mb:.2} MB, {} batches; GPU {:.1} ms ({:.2} GB/s): first {:.1}, again {:.1}, positions {:.1}; {} launches, {} rounds; host {:.1?} CPU, {:.1?} wall",
                sum[5], gpu * 1e3, mb / 1e3 / gpu, sum[0] * 1e3, sum[1] * 1e3, sum[2] * 1e3, sum[3], sum[4], cpu, wall
            );
        }
        eprintln!("made during the timed runs: {}", compiles() - before);
        eprintln!("load: {}", String::from_utf8_lossy(&std::process::Command::new("uptime").output().map(|o| o.stdout).unwrap_or_default()).trim());
    }

    /// Tier 2 in clusters (`native_cpu_clusters`, doc 28) on `.rr`: the traffic from every
    /// fourth file of the corpus, a cut at each cap, and for each the compile (CPU, Cranelift's
    /// register allocation), the code (instructions, stack loads and stores), the share of steps
    /// that cross, and tier 2's time over the whole corpus with tier 1's buffers made before.
    /// Every file's table equals the one function's. Run by hand, at nice 19.
    /// The compiled grammar against the engine's Rust parser (`rr::tape`), over the corpus: tier
    /// 1 and tier 2 on buffers made before, the Rust parser from the text. Instructions retired,
    /// cycles and CPU time, each the least of the rounds, the parsers in turns. Build the Rust
    /// parser optimized for a fair count:
    /// `--config 'profile.dev.package.rerail-rr-syntax.opt-level=3'`.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_against_rust() {
        let c = crate::compiled().expect("compiles");
        let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
        let files = corpus();
        let ours: Vec<&String> = files.iter().filter(|f| crate::compiled_tape(f).is_some()).collect();
        let bytes: usize = ours.iter().map(|f| f.len()).sum();
        let p = &c.parser;
        let mut batches: Vec<_> = ours.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_bytes()])).collect();
        for b in &mut batches {
            b.call(&p.tier1);
        }
        let mut tapes: Vec<_> = batches.iter().map(|b| p.tape(b)).collect();
        let nodes: usize = ours.iter().map(|f| super::rr::tape(f).len()).sum();
        let retired = rerail_compiler::native_cpu_codegen::retired;
        // tier 1, tier 2, the Rust parser, the lean parser's lexer, the lean parser whole, the
        // lean parser's first lexer (a byte at a time)
        let mut best = [(std::time::Duration::MAX, u64::MAX, u64::MAX); 6];
        let mut keep = |k: usize, t: std::time::Duration, i: u64, c: u64| best[k] = (best[k].0.min(t), best[k].1.min(i), best[k].2.min(c));
        for _ in 0..rounds {
            let (i0, c0, t0) = (retired().0, retired().1, thread_cpu());
            for b in &mut batches {
                b.call(&p.tier1);
            }
            let (t1, (i1, c1)) = (thread_cpu(), retired());
            for (b, t) in batches.iter().zip(&mut tapes) {
                let _ = p.words(b, t);
            }
            let (t2, (i2, c2)) = (thread_cpu(), retired());
            let mut n = 0;
            for f in &ours {
                n += super::rr::tape(f).len();
            }
            std::hint::black_box(n);
            let (t3, (i3, c3)) = (thread_cpu(), retired());
            for f in &ours {
                n += super::rr::lean::token_count(f);
            }
            let (t4, (i4, c4)) = (thread_cpu(), retired());
            for f in &ours {
                n += super::rr::lean::tape(f).len();
            }
            std::hint::black_box(n);
            let (t5, (i5, c5)) = (thread_cpu(), retired());
            for f in &ours {
                n += super::rr::lean_scalar::token_count(f);
            }
            std::hint::black_box(n);
            let (t6, (i6, c6)) = (thread_cpu(), retired());
            keep(5, t6 - t5, i6 - i5, c6 - c5);
            keep(3, t4 - t3, i4 - i3, c4 - c3);
            keep(4, t5 - t4, i5 - i4, c5 - c4);
            keep(0, t1 - t0, i1 - i0, c1 - c0);
            keep(1, t2 - t1, i2 - i1, c2 - c1);
            keep(2, t3 - t2, i3 - i2, c3 - c2);
        }
        eprintln!("{} files the grammar parses, {:.2} MB, {nodes} nodes; the least of {rounds} rounds", ours.len(), bytes as f64 / 1e6);
        for (name, (t, i, cy)) in ["tier 1", "tier 2", "Rust (rr::tape)", "lean: lexer", "lean: whole", "lean: byte lexer"].iter().zip(best) {
            eprintln!("  {name:>16}: {t:8.2?} CPU, {:7.1}M instructions, {:7.1}M cycles, IPC {:.2}; {:.0} instructions and {:.0} cycles a node", i as f64 / 1e6, cy as f64 / 1e6, i as f64 / cy as f64, i as f64 / nodes as f64, cy as f64 / nodes as f64);
        }
        // Where tier 2's instructions go: each word's Cranelift instructions times how often the
        // corpus runs it (the interpreter's steps), by the word's operation, a token.
        let native = rerail_compiler::native_ir::lower(&c.program, 0, 0).expect("lowers");
        let all: Vec<&[u8]> = ours.iter().map(|f| f.as_bytes()).collect();
        let traffic = rerail_compiler::native_cpu_clusters::traffic(&c.program, &all);
        let mut runs = vec![0u64; native.structural.len()];
        for (&(_, b), &w) in &traffic {
            runs[b as usize] += w;
        }
        let insts = &p.tier2.word_insts;
        let tokens: u64 = (0..runs.len()).filter(|&pc| matches!(native.structural[pc].operation, rerail_compiler::native_ir::StructuralOperation::Token { .. })).map(|pc| runs[pc]).sum();
        let mut by: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
        for pc in 0..runs.len() {
            let op = format!("{:?}", native.structural[pc].operation);
            let e = by.entry(op.split([' ', '{', '(']).next().unwrap_or("").to_owned()).or_default();
            e.0 += runs[pc];
            e.1 += runs[pc] * u64::from(insts.get(pc).copied().unwrap_or(0));
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        let total: u64 = v.iter().map(|x| x.1.1).sum();
        let pushes = rerail_compiler::native_cpu_stream::push_targets(&native);
        // The switches as run: their runs of bytes, weighted by how often each switch runs.
        let (mut sw_runs, mut sw_n) = (0u64, 0u64);
        for pc in 0..runs.len() {
            if let rerail_compiler::native_ir::StructuralOperation::Switch { table } = native.structural[pc].operation {
                let t = &native.switches[usize::from(table)].targets;
                let n = 1 + t.windows(2).filter(|w| w[0] != w[1]).count() as u64;
                sw_runs += runs[pc] * n;
                sw_n += runs[pc];
            }
        }
        eprintln!("{} places a RETURN can go back to (a compare tree {} deep); a switch as run has {:.1} runs of bytes ({:.1} compares deep)", pushes.len(), (pushes.len() as f64).log2().ceil(), sw_runs as f64 / sw_n.max(1) as f64, (sw_runs as f64 / sw_n.max(1) as f64).log2());
        eprintln!("tier 2's IR instructions run, {:.0} a token ({tokens} tokens), by word:", total as f64 / tokens as f64);
        for (k, (n, i)) in v.iter().take(16) {
            eprintln!("  {k:>10}: {:5.2} words a token, {:5.1} instructions each, {:6.1} a token", *n as f64 / tokens as f64, *i as f64 / (*n).max(1) as f64, *i as f64 / tokens as f64);
        }
    }

    /// One form over the corpus, again and again for `RR_SECONDS` (10) of CPU, for a profiler
    /// (`xctrace record --template 'Time Profiler' --launch -- <binary> rr_profile_loop
    /// --ignored`). `RR_FORM`: `lean` (the lean parser whole, by default), `lexer` (its lexer),
    /// `tier1` or `tier2` (the compiled grammar's, on buffers made before). Build as
    /// `rr_against_rust` says.
    #[test]
    #[ignore = "a profile's load: run by hand"]
    fn rr_profile_loop() {
        let form = std::env::var("RR_FORM").unwrap_or_else(|_| "lean".into());
        let seconds: f64 = std::env::var("RR_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(10.0);
        let c = crate::compiled().expect("compiles");
        let files = corpus();
        let ours: Vec<&String> = files.iter().filter(|f| crate::compiled_tape(f).is_some()).collect();
        let p = &c.parser;
        // `RR_DUMP=dir`: tier 2's CLIF and machine code there, to read.
        if let Ok(dir) = std::env::var("RR_DUMP") {
            std::fs::write(format!("{dir}/tier2.clif"), &p.tier2.clif).expect("writes");
            std::fs::write(format!("{dir}/tier2.s"), &p.tier2.disassembly).expect("writes");
            std::fs::write(format!("{dir}/tier2.bin"), &p.tier2.code_bytes).expect("writes");
            std::fs::write(format!("{dir}/tier2.words"), &c.words).expect("writes");
            // Each word's Cranelift instructions as lowered, a line a place.
            let insts: String = p.tier2.word_insts.iter().map(|n| format!("{n}\n")).collect();
            std::fs::write(format!("{dir}/tier2.insts"), insts).expect("writes");
            let bytes: String = p.tier2.word_bytes.iter().map(|n| format!("{n}\n")).collect();
            std::fs::write(format!("{dir}/tier2.bytes"), bytes).expect("writes");
            let ranges: String = p.tier2.word_ranges.iter().map(|(pc, a, b)| format!("{pc} {a} {b}\n")).collect();
            std::fs::write(format!("{dir}/tier2.ranges"), ranges).expect("writes");
            std::fs::write(format!("{dir}/tier2.machine"), &c.machine).expect("writes");
        }
        let mut batches: Vec<_> = ours.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_bytes()])).collect();
        for b in &mut batches {
            b.call(&p.tier1);
        }
        let mut tapes: Vec<_> = batches.iter().map(|b| p.tape(b)).collect();
        let retired = rerail_compiler::native_cpu_codegen::retired;
        let (t0, (i0, c0)) = (thread_cpu(), retired());
        let (mut rounds, mut n) = (0usize, 0usize);
        while (thread_cpu() - t0).as_secs_f64() < seconds {
            match form.as_str() {
                "lean" => ours.iter().for_each(|f| n += super::rr::lean::tape(f).len()),
                "lexer" => ours.iter().for_each(|f| n += super::rr::lean::token_count(f)),
                "tier1" => batches.iter_mut().for_each(|b| b.call(&p.tier1)),
                "tier2" => batches.iter().zip(&mut tapes).for_each(|(b, t)| {
                    let _ = p.words(b, t);
                }),
                other => panic!("RR_FORM={other}: lean, lexer, tier1 or tier2"),
            }
            rounds += 1;
        }
        std::hint::black_box(n);
        let (t1, (i1, c1)) = (thread_cpu(), retired());
        eprintln!("{form}: {rounds} rounds, {:.1}M instructions and {:.1}M cycles a round, {:?} CPU", (i1 - i0) as f64 / rounds as f64 / 1e6, (c1 - c0) as f64 / rounds as f64 / 1e6, (t1 - t0) / rounds as u32);
    }

    /// Cranelift's optimizer on `.rr`'s tier 1 (doc 28): tier 1 compiled with it and without, in
    /// one process, timed in turns over the corpus, each its best round.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_tier1_optimizer() {
        use std::sync::atomic::Ordering::Relaxed;
        let c = crate::compiled().expect("compiles");
        let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
        let tier1: Vec<(&str, rerail_compiler::native_cpu_block::CompiledBlock)> = [("off", false), ("on", true)]
            .into_iter()
            .map(|(label, on)| {
                rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER1.store(on, Relaxed);
                (label, rerail_compiler::native_cpu_block::compile(&c.native).expect("tier 1"))
            })
            .collect();
        rerail_compiler::native_cpu_codegen::OPTIMIZE_TIER1.store(false, Relaxed);
        let files = corpus();
        let mut batches: Vec<Vec<_>> = tier1.iter().map(|(_, t)| files.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(t, &[f.as_bytes()])).collect()).collect();
        let mut best = vec![(std::time::Duration::MAX, u64::MAX, u64::MAX); tier1.len()];
        for _ in 0..rounds {
            for (k, (_, t)) in tier1.iter().enumerate() {
                let (i0, c0) = rerail_compiler::native_cpu_codegen::retired();
                let s = thread_cpu();
                for b in &mut batches[k] {
                    b.call(t);
                }
                let s = thread_cpu() - s;
                let (i1, c1) = rerail_compiler::native_cpu_codegen::retired();
                best[k] = (best[k].0.min(s), best[k].1.min(i1 - i0), best[k].2.min(c1 - c0));
            }
        }
        let bytes: usize = files.iter().map(String::len).sum();
        for ((label, _), (t, ins, cyc)) in tier1.iter().zip(&best) {
            eprintln!("tier 1, optimizer {label}: {t:.2?} CPU, {:.1}M instructions, {:.1}M cycles, IPC {:.2} over {} files ({:.2} GB/s), the best of {rounds}", *ins as f64 / 1e6, *cyc as f64 / 1e6, *ins as f64 / *cyc as f64, files.len(), bytes as f64 / t.as_nanos() as f64);
        }
    }

    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_tier2_clusters() {
        use rerail_compiler::native_cpu_clusters::{by_traffic, crossings, traffic, Cut};
        use rerail_compiler::native_cpu_stream::Parser;
        let c = super::compiled().expect("compiles");
        let native = rerail_compiler::native_ir::lower(&c.program, 0, 0).expect("lowers");
        let files = corpus();
        let every: usize = std::env::var("RR_PROFILE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
        let sample: Vec<&[u8]> = files.iter().step_by(every).map(|f| f.as_bytes()).collect();
        let t = thread_cpu();
        let traffic = traffic(&c.program, &sample);
        eprintln!("{} words; the profile of {} files: {:.2?} CPU", native.structural.len(), sample.len(), thread_cpu() - t);
        {
            // Words run a token, by kind: what a token's ~100 cycles pay for.
            let tokens: u64 = traffic.iter().filter(|((_, b), _)| matches!(native.structural[*b as usize].operation, rerail_compiler::native_ir::StructuralOperation::Token { .. })).map(|(_, w)| w).sum();
            let mut by: std::collections::BTreeMap<String, u64> = Default::default();
            for (&(_, b), &w) in &traffic {
                let op = format!("{:?}", native.structural[b as usize].operation);
                *by.entry(op.split([' ', '{', '(']).next().unwrap_or("").to_owned()).or_default() += w;
            }
            let all: u64 = by.values().sum();
            let mut v: Vec<_> = by.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            eprintln!("{:.2} words a token: {}", all as f64 / tokens.max(1) as f64, v.iter().take(14).map(|(k, w)| format!("{k} {:.2}", *w as f64 / tokens.max(1) as f64)).collect::<Vec<_>>().join(", "));
        }
        // The whole corpus's traffic, for the crossings off the sample too.
        let all: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        // `RR_ONLY=form`: that form alone, its crossings over the sample only (a profile of its
        // run, `xctrace`'s counters, with little else in the trace).
        let only = std::env::var("RR_ONLY").ok();
        let whole = if only.is_some() { traffic.clone() } else { rerail_compiler::native_cpu_clusters::traffic(&c.program, &all) };
        let caps: Vec<usize> = std::env::var("RR_CAPS").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![600, 1200, 2500]);
        // By the traffic, and by the program alone (no profile: its own edges, in order of place).
        let none = std::collections::HashMap::new();
        let mut shapes: Vec<(String, Cut)> = std::iter::once(("one".to_owned(), Cut::one(native.structural.len()))).chain(caps.iter().map(|&cap| (format!("cap {cap}"), by_traffic(&native, &traffic, cap)))).collect();
        shapes.push(("rules".to_owned(), Cut { speed: true, ..rerail_compiler::native_cpu_clusters::by_rules(&native) }));
        if std::env::var_os("RR_STATES").is_some() {
            shapes.push(("states".to_owned(), rerail_compiler::native_cpu_clusters::by_states(&native)));
        }
        if std::env::var_os("RR_STATIC").is_some() {
            shapes.extend(caps.iter().map(|&cap| (format!("static {cap}"), by_traffic(&native, &none, cap))));
        }
        // The state's fields held in registers (`Cut::held`, by `Mem`'s order): none; the
        // token's (next, start, end, limit, last end, at end, need, live); those and the
        // tree's and the mask's (taken, lw, nodes, open, ret, pending); every field.
        let sets: [(&str, Vec<u32>); 4] = [("memory", vec![]), ("token", vec![10, 13, 14, 15, 16, 17, 21, 22]), ("tree", vec![10, 13, 14, 15, 16, 17, 21, 22, 11, 12, 18, 19, 20, 1]), ("all", (0..23).collect())];
        let wanted: Vec<String> = std::env::var("RR_HELD").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_else(|_| sets.iter().map(|s| s.0.to_owned()).collect());
        let cuts: Vec<(String, Cut)> = shapes
            .iter()
            .flat_map(|(name, cut)| sets.iter().filter(|s| wanted.iter().any(|w| w == s.0)).map(move |(h, held)| (format!("{name}/{h}"), Cut { held: held.clone(), ..cut.clone() })))
            // `Cut::reload`: each word loads the `ctx` fields it reads.
            .flat_map(|(name, cut)| {
                let at_word = std::env::var_os("RR_RELOAD").is_some().then(|| (format!("{name}+ctx"), Cut { reload: true, ..cut.clone() }));
                std::iter::once((name, cut)).chain(at_word)
            })
            // `Cut::inline_span`: `SPAN`'s common case inline, no call.
            .flat_map(|(name, cut)| {
                let inline = std::env::var_os("RR_SPAN").is_some().then(|| (format!("{name}+span"), Cut { inline_span: true, ..cut.clone() }));
                std::iter::once((name, cut)).chain(inline)
            })
            // `Cut::pass`: next, start, end and need as the tail calls' arguments, where held.
            .flat_map(|(name, cut)| {
                let pass = vec![10, 13, 14, 21];
                let passed = (std::env::var_os("RR_PASS").is_some() && pass.iter().all(|k| cut.held.contains(k))).then(|| (format!("{name}+pass"), Cut { pass, ..cut.clone() }));
                std::iter::once((name, cut)).chain(passed)
            })
            // `Cut::speed`: Cranelift's optimizer on.
            .flat_map(|(name, cut)| {
                let fast = std::env::var_os("RR_SPEED").is_some().then(|| (format!("{name}+opt"), Cut { speed: true, ..cut.clone() }));
                std::iter::once((name, cut)).chain(fast)
            })
            // `Cut::near` and `unsigned`: a crossing a direct branch, return addresses not signed,
            // entries picked by the profile.
            .flat_map(|(name, cut)| {
                // `RR_NEAR`: all three, or `direct`, `unsigned`, `pick` alone (comma-separated).
                let parts = std::env::var("RR_NEAR").ok();
                let near: Vec<(String, Cut)> = parts
                    .iter()
                    .flat_map(|p| p.split(','))
                    .map(|p| match p {
                        "direct" => (format!("{name}+direct"), Cut { near: true, ..cut.clone() }),
                        "unsigned" => (format!("{name}+unsigned"), Cut { unsigned: true, ..cut.clone() }),
                        "pick" => (format!("{name}+pick"), Cut { pick_traffic: traffic.clone(), ..cut.clone() }),
                        _ => (format!("{name}+near"), Cut { near: true, unsigned: true, pick_traffic: traffic.clone(), ..cut.clone() }),
                    })
                    .collect();
                std::iter::once((name, cut)).chain(near)
            })
            // `Cut::pick`: the switches weighted by the profile, or by a table.
            .flat_map(|(name, cut)| {
                let weighted = (name.clone(), Cut { pick: rerail_compiler::native_cpu_clusters::Pick::Weighted, pick_traffic: traffic.clone(), ..cut.clone() });
                let table = (name.clone(), Cut { pick: rerail_compiler::native_cpu_clusters::Pick::Table, ..cut.clone() });
                let more = std::env::var_os("RR_ARMS").is_some();
                std::iter::once((name.clone(), cut)).chain(more.then(|| (format!("{}+weighted", weighted.0), weighted.1))).chain(more.then(|| (format!("{}+table", table.0), table.1)))
            })
            .filter(|(name, _)| only.as_ref().is_none_or(|o| o.split(',').any(|o| o == name)))
            .collect();
        // Tier 1's buffers, a batch a file, made once.
        let one = &c.parser;
        let batches: Vec<_> = files
            .iter()
            .map(|f| {
                let mut b = rerail_compiler::native_cpu_block::Batch::new(&one.tier1, &[f.as_bytes()]);
                b.call(&one.tier1);
                b
            })
            .collect();
        let want: Vec<_> = batches.iter().map(|b| {
            let mut tape = one.tape(b);
            one.words(b, &mut tape).map(|_| tape.columns()).map_err(|r| r.pc)
        }).collect();
        let asm = |text: &str| {
            let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.ends_with(':') && !l.starts_with(';')).collect();
            (lines.len(), lines.iter().filter(|l| ["[sp", "[fp", "spillslot"].iter().any(|p| l.contains(p))).count())
        };
        let mut made = Vec::new();
        for (name, cut) in &cuts {
            let _ = cranelift_codegen::timing::take_current();
            // Tier 2's compile alone (tier 1's is the same for every form).
            // Its CPU over every thread (the process's: run alone), and its wall time.
            let _ = rerail_compiler::native_cpu_codegen::parallel_pass_times();
            let (t, w) = (cpu(), std::time::Instant::now());
            let tier2 = rerail_compiler::native_cpu_stream::compile_cut(&native, cut).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let (compile, compile_wall) = (cpu() - t, w.elapsed());
            let mut times = cranelift_codegen::timing::take_current();
            times.add(&rerail_compiler::native_cpu_codegen::parallel_pass_times());
            let timing = times.to_string();
            let p = Parser { tier1: rerail_compiler::native_cpu_block::compile(&native.block_programs[0]).expect("tier 1"), tier2 };
            if std::env::var_os("RR_TIMING").is_some() {
                eprintln!("{name}:\n{timing}");
            }
            let regalloc = timing.lines().find(|l| l.contains("Register allocation")).map(|l| l.split_whitespace().next().unwrap_or("?").to_owned()).unwrap_or_default();
            let (code, stack) = asm(&p.tier2.disassembly);
            if let Ok(dir) = std::env::var("RR_DUMP") {
                let tag = name.replace([' ', '/', '+'], "-");
                std::fs::write(format!("{dir}/tier2-{tag}.s"), &p.tier2.disassembly).expect("writes");
                std::fs::write(format!("{dir}/tier2-{tag}.clif"), &p.tier2.clif).expect("writes");
            }
            // The state's own loads and stores, in one function's stack frame: not spills.
            let state = p.tier2.clif.lines().filter(|l| l.contains("stack_load") || l.contains("stack_store")).count();
            for (k, b) in batches.iter().enumerate() {
                let mut tape = p.tape(b);
                let got = p.words(b, &mut tape).map(|_| tape.columns()).map_err(|r| r.pc);
                assert!(got == want[k], "{name}: file {k} differs from the one function's");
            }
            eprintln!(
                "{name:>16}: {:>3} functions, {:>4} tail calls, {:.3} of the sample's steps cross ({:.4} of the corpus's); compile {compile:.2?} CPU, {compile_wall:.2?} wall (register allocation {regalloc} s CPU); {code} instructions, {} stack loads/stores, {state} of them the state's",
                p.tier2.clusters, p.tier2.crossings, crossings(&native, cut, &traffic), crossings(&native, cut, &whole), stack
            );
            made.push((name, p));
        }
        // Tier 2's time over the corpus, the forms in turn (the machine's load moves): each
        // form's best of the rounds.
        let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
        // CPU time, and the instructions retired and cycles (each its least over the rounds).
        let mut best = vec![(std::time::Duration::MAX, u64::MAX, u64::MAX); made.len()];
        let mut tapes: Vec<Vec<_>> = made.iter().map(|(_, p)| batches.iter().map(|b| p.tape(b)).collect()).collect();
        for _ in 0..rounds {
            for (i, (_, p)) in made.iter().enumerate() {
                let (i0, c0) = rerail_compiler::native_cpu_codegen::retired();
                let t = thread_cpu();
                for (b, tape) in batches.iter().zip(&mut tapes[i]) {
                    let _ = p.words(b, tape);
                }
                let t = thread_cpu() - t;
                let (i1, c1) = rerail_compiler::native_cpu_codegen::retired();
                best[i] = (best[i].0.min(t), best[i].1.min(i1 - i0), best[i].2.min(c1 - c0));
            }
        }
        for ((name, _), (t, ins, cyc)) in made.iter().zip(&best) {
            eprintln!("{name:>16}: tier 2 {t:.1?} CPU, {:.1}M instructions, {:.1}M cycles, IPC {:.2} over the corpus (the best of {rounds})", *ins as f64 / 1e6, *cyc as f64 / 1e6, *ins as f64 / *cyc as f64);
        }
        eprintln!("load: {}", String::from_utf8_lossy(&std::process::Command::new("uptime").output().map(|o| o.stdout).unwrap_or_default()).trim());
    }

    /// Throughput over the repository's `.rr` files, a pass's mean of `REPEAT`, CPU (this thread)
    /// and wall: the hand-written Rust reader (`rr::tape`), the compiled grammar file by file,
    /// the corpus as one batch with tier 1 on the CPU, and as one batch with tier 1 on Metal
    /// (tier 2 on the CPU over the buffers where the device left them). Each pass's results are
    /// checked against the file-by-file tables. Run alone, at nice 19; the load is printed. For
    /// the reader's speed, build `rerail-rr-syntax` optimized
    /// (`--config 'profile.dev.package.rerail-rr-syntax.opt-level=3'`); the generated code is the
    /// same in any build.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_throughput() {
        use rerail_compiler::native_cpu_block::Batch;
        const REPEAT: u32 = 3;
        let c = super::compiled().expect("compiles");
        let p = &c.parser;
        // `RR_REPEAT=n`: the corpus n times over, as one batch.
        let n: usize = std::env::var("RR_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let files: Vec<String> = (0..n).flat_map(|_| corpus()).collect();
        let refs: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        let mb = refs.iter().map(|b| b.len()).sum::<usize>() as f64 / 1e6;
        let want: Vec<_> = files.iter().map(|f| super::tape(f)).collect();
        let tokens: usize = files.iter().map(|f| super::rr::tape(f).len()).sum();
        // (CPU, wall) a pass of `f`, its last pass's tables checked.
        let time = |warm: usize, f: &mut dyn FnMut() -> Option<Vec<super::Columns>>| {
            let w = std::time::Instant::now();
            for _ in 0..warm {
                let _ = f();
            }
            eprintln!("    ({warm} calls first: {:.2?} wall)", w.elapsed());
            let (t, w) = (thread_cpu(), std::time::Instant::now());
            let mut last = None;
            for _ in 0..REPEAT {
                last = f();
            }
            let r = ((thread_cpu() - t) / REPEAT, w.elapsed() / REPEAT);
            if let Some(got) = last {
                assert_eq!(got.len(), want.len());
                assert!(got == want, "a pass's tables differ from the file-by-file ones");
            }
            r
        };
        let row = |name: &str, (cpu, wall): (std::time::Duration, std::time::Duration), extra: &str| {
            eprintln!("  {name:<44} {:>8.1}ms {:>8.1}ms {:>8.1} {extra}", cpu.as_secs_f64() * 1e3, wall.as_secs_f64() * 1e3, mb / cpu.as_secs_f64());
        };
        eprintln!("{} files, {mb:.2} MB, {} reader nodes", files.len(), tokens);
        eprintln!("  {:<44} {:>10} {:>10} {:>8}", "", "CPU/pass", "wall/pass", "MB/s CPU");
        #[cfg(target_os = "macos")]
        {
            use rerail_metal::block_program::serial::Resident;
            // Tier 1 on Metal over `files`, halved while the device refuses it for capacity (as
            // `metal_tapes`), each part's buffers given to `each` as they come (held all at once,
            // they pass the device's budget): its GPU seconds, its first file, its buffers (none:
            // a file past the device's room alone).
            fn parts(c: &super::Compiled, files: &[rerail_compiler::block::BatchFile<'_>], digests: &[[u8; 32]], at: usize, each: &mut dyn FnMut(f64, usize, usize, Option<&Resident>)) {
                let (t, w) = (thread_cpu(), std::time::Instant::now());
                let r = rerail_metal::block_program::serial::run_resident_digested(&[&c.native], files, Some(digests), super::SEGMENTS);
                if std::env::var_os("RR_PARTS").is_some() {
                    eprintln!("      run_resident {} files: {:.2?} CPU {:.2?} wall {}", files.len(), thread_cpu() - t, w.elapsed(), if r.is_ok() { "ok" } else { "refused" });
                }
                match r {
                    Ok((run, res)) => {
                        let (t, w) = (thread_cpu(), std::time::Instant::now());
                        each(run.run.seconds, at, files.len(), Some(&res));
                        let (tc, tw) = (thread_cpu(), std::time::Instant::now());
                        drop(res);
                        if std::env::var_os("RR_PARTS").is_some() {
                            eprintln!("      tier 2 {:.2?} CPU {:.2?} wall; drop {:.2?} CPU {:.2?} wall", tc - t, tw - w, thread_cpu() - tc, tw.elapsed());
                        }
                    }
                    Err(rerail_compiler::Error::BackendCapacity) if files.len() > 1 => {
                        let (a, b) = files.split_at(files.len() / 2);
                        let (da, db) = digests.split_at(files.len() / 2);
                        parts(c, a, da, at, each);
                        parts(c, b, db, at + a.len(), each);
                    }
                    Err(rerail_compiler::Error::BackendCapacity) => each(0.0, at, 1, None),
                    Err(e) => panic!("Metal runs the batch: {e:?}"),
                }
            }
            let batch: Vec<_> = refs.iter().map(|b| rerail_compiler::block::BatchFile { bytes: b, program: 0 }).collect();
            // The files' digests, made once: the engine has them from a `stat` or a Git blob.
            let digests: Vec<[u8; 32]> = refs.iter().map(|b| <sha2::Sha256 as sha2::Digest>::digest(b).into()).collect();
            let (mut gpu, mut t2_cpu, mut n_parts, mut on_cpu) = (0.0, std::time::Duration::ZERO, 0, 0);
            // Three calls first: the second runs a program its first run learned from (its source
            // made and compiled again), the third the settled one.
            let metal = time(3, &mut || {
                let mut out: Vec<Option<super::Columns>> = vec![None; want.len()];
                let (mut np, mut oc) = (0, 0);
                parts(c, &batch, &digests, 0, &mut |secs, at, n, res| {
                    gpu += secs;
                    np += 1;
                    let t = thread_cpu();
                    for i in 0..n {
                        out[at + i] = Some(match res {
                            Some(res) => rerail_metal::tier2_on::columns_on(p, res, i).unwrap_or_else(|_| want[at + i].clone()),
                            None => {
                                oc += 1;
                                super::tape(&files[at + i])
                            }
                        });
                    }
                    t2_cpu += thread_cpu() - t;
                });
                (n_parts, on_cpu) = (np, oc);
                Some(out.into_iter().map(Option::unwrap_or_default).collect())
            });
            row("one batch, tier 1 on Metal, tier 2 on the CPU", metal, &format!("({n_parts} batches; {on_cpu} files too big, on the CPU)"));
            let passes = f64::from(REPEAT + 3);
            eprintln!("    tier 1's GPU time {:.1}ms a pass ({:.2} GB/s); tier 2 on the CPU {:.1}ms", gpu / passes * 1e3, mb / 1e3 / (gpu / passes), t2_cpu.as_secs_f64() / passes * 1e3);
            let t1m = time(1, &mut || {
                parts(c, &batch, &digests, 0, &mut |_, _, _, res| {
                    std::hint::black_box(res);
                });
                None
            });
            row("  tier 1 alone, one Metal batch", t1m, "");
        }
        // The reader once over the corpus (the batch repeats it): its rate is the same.
        if n == 1 {
        let reader = time(1, &mut || {
            let n: usize = files.iter().map(|f| super::rr::tape(f).len()).sum();
            std::hint::black_box(n);
            None
        });
        row("Rust reader (rr::tape), file by file", reader, "");
        }
        let by_file = time(1, &mut || Some(files.iter().map(|f| super::tape(f)).collect()));
        row("compiled, file by file", by_file, "");
        // Tier 1 as one batch on the CPU; tier 2 file by file (it runs one file at a time).
        let t1 = time(1, &mut || {
            let mut b = Batch::new(&p.tier1, &refs);
            b.call(&p.tier1);
            std::hint::black_box(&b);
            None
        });
        row("  tier 1 alone, one CPU batch", t1, "");
        let load = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
        eprintln!("  load: {load}");
    }

    /// CPU time over the repository's `.rr` files: the compiled grammar against the engine's
    /// parser, each over every file `REPEAT` times, the grammar compiled before. Run alone
    /// (`--test-threads 1`) and at nice 19; the machine's load is printed.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_parser_speed() {
        const REPEAT: usize = 5;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
    pub fn corpus() -> Vec<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
        use rerail_compiler::native_cpu_block::Batch;
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
        // The tape's room against what is written: rows allocated (not zeroed) for every file.
        let (mut room, mut used) = (0usize, 0usize);
        for src in &files {
            let mut batch = Batch::new(&p.tier1, &[src.as_bytes()]);
            batch.call(&p.tier1);
            let mut tape = p.tape(&batch);
            room += tape.rows.capacity();
            if p.words(&batch, &mut tape).is_ok() {
                used += tape.counts[0] as usize;
            }
        }
        let retries = rerail_compiler::native_cpu_stream::ROOM_RETRIES.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!("tape rows: {room} allocated, {used} written ({:.1}x), {} MB of room a pass; {retries} runs again on the bound's tape (all passes)", room as f64 / used.max(1) as f64, room * 24 / 1_000_000);
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
        use rerail_compiler::native_cpu_pressure::{INT_REGS, VEC_REGS};
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
            let loops: Vec<&rerail_compiler::native_cpu_pressure::Blame> = q.blame.iter().filter(|b| b.origin.is_none()).collect();
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
