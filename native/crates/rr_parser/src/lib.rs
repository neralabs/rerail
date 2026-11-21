//! The `.rerail` parser: the Bitlex grammar of `.rerail` (`rr.bitlex`), compiled
//! once per process through the Bitlex compiler to tier 1 and tier 2 on Cranelift, and shared by
//! every thread.
//!
//! A crate of its own, on the compiler crate, the Metal crate and the engine's `.rerail` syntax
//! (`rerail_rr_syntax`), so the parser and its measurements rebuild without the engine.
//!
//! `tape` gives the engine's own tape (`rerail_rr_syntax::tape`): the same kinds, fields, spans and
//! parents. The grammar's kinds and fields take the engine's ids by name, so no table maps them.
//!
//! A file the compiled parser refuses goes to the engine's parser (`rerail_rr_syntax::tape`). On the
//! corpus that is a file the engine refuses too, or a pathological one the grammar leaves to it
//! on purpose: a run of four backticks or more, braces nested past 127.

use rerail_compiler::bitlex_masks as bitlex_compile;
use rerail_compiler::{native_cpu_stream, native_ir, rir_asm as rir};
use rerail_rr_syntax as rr;
use std::cell::RefCell;
use std::collections::HashMap;

/// The grammar: the ladder's last step.
const GRAMMAR: &str = include_str!("../rr.bitlex");

/// Sample documents, for the profile tier 2 is cut by when the grammar compiles
/// (docs/parser-generators/28-tier-2-in-clusters.md): 12 of the repository's `.rerail`
/// files, spread over its folders. A profile of about ten files gives a cut as good as the
/// whole corpus's.
const SAMPLES: [&str; 12] = [
    include_str!("../samples/data-layer-graph-lazy-sql-plan.rerail.sample"),
    include_str!("../samples/eslint-no-sequences.rerail.sample"),
    include_str!("../samples/node.rerail.sample"),
    include_str!("../samples/phpcs-generic-php-forbidden-functions.rerail.sample"),
    include_str!("../samples/plan-scope-side-effects.rerail.sample"),
    include_str!("../samples/react-hooks-rules-of-hooks.rerail.sample"),
    include_str!("../samples/ruff-b029-except-with-empty-tuple.rerail.sample"),
    include_str!("../samples/ruff-fbt001-boolean-type-hint-positional-argument.rerail.sample"),
    include_str!("../samples/ruff-pth115-os-readlink.rerail.sample"),
    include_str!("../samples/ruff-ruf043-pytest-raises-ambiguous-pattern.rerail.sample"),
    include_str!("../samples/swallowed-exceptions.rerail.sample"),
    include_str!("../samples/typescript-eslint-no-dynamic-delete.rerail.sample"),
];

/// The cap of tier 2's clusters, in words (doc 28: 2,500 runs fastest on `.rerail`).
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
    compile_text(GRAMMAR, shape)
}

/// `compile_with` of a grammar's text (a test's variant of the `.rerail` grammar).
pub fn compile_text(grammar: &str, shape: bitlex_compile::Shape) -> Result<Compiled, String> {
    compile_padded(grammar, shape, rerail_compiler::native_cpu_block::BACK as u16)
}

/// `compile_text`, the code counting on `pad` zero bytes after every file (`BlockNative::pad`,
/// the application's input): with fewer than a window reaches, each window is cut on a rare
/// branch.
pub fn compile_padded(grammar: &str, shape: bitlex_compile::Shape, pad: u16) -> Result<Compiled, String> {
    let stmts = ::bitlex::syntax::parse(grammar)?;
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
    let mut native = native_ir::lower(&program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?;
    native.tag = rerail_tape_format::RR_TAG;
    for b in &mut native.block_programs {
        b.pad = pad;
    }
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

/// `src`'s tape, the engine's bytes (decision 0140): from the compiled grammar, or, for a file
/// the grammar refuses, from the engine's parser (`logic::rr::tape`) through the tape's writer.
pub fn tape(src: &str) -> Vec<u8> {
    compiled_tape(src).unwrap_or_else(|| engine_tape(src))
}

/// The compiled grammar's tape, or `None` where it refuses the file (or did not compile). Every
/// byte is the compiled code's, each column written by tier 2's words at its place as it
/// parses: no pass after the parse (docs/parser-generators/37).
pub fn compiled_tape(src: &str) -> Option<Vec<u8>> {
    compiled().ok()?.parser.image(src.as_bytes()).ok()
}

/// The tapes of `files`, in order, as `tape` gives each: tier 1 over the whole batch on
/// the device (Metal's forward carry), then tier 2 over each file reading tier 1's buffers where
/// the device left them; without a device, `tape` file by file.
pub fn tapes(files: &[&str]) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(files.len());
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SEGMENTS: rerail_metal::block_program::serial::Segments = rerail_metal::block_program::serial::Segments { blocks: 29, warm: 1 };

/// `tapes` on Metal: halves while the device refuses the batch for capacity; `false` when it
/// cannot run it at all.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn metal_tapes(c: &Compiled, files: &[&str], out: &mut Vec<Vec<u8>>) -> bool {
    use rerail_metal::block_program::serial::run_resident;
    let mut batch = Vec::with_capacity(files.len());
    for file in files {
        batch.push(rerail_compiler::block::BatchFile { bytes: file.as_bytes(), program: 0 });
    }
    match run_resident(&[&c.native], &batch, SEGMENTS) {
        Ok((_, run)) => {
            let mut i = 0;
            for file in files {
                out.push(rerail_metal::tier2_on::image_on(&c.parser, &run, i).unwrap_or_else(|_| engine_tape(file)));
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

/// The fallback: the engine's parser's nodes through the tape's writer, its bytes.
pub fn engine_tape(src: &str) -> Vec<u8> {
    rr::tape::image(&rr::engine_tape(src), src)
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
    /// No device here that runs a block program (none at all, or SIMD groups not of 32 lanes:
    /// `block_program::ready`): a test's device half is left out, as the executors' are.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn no_device() -> bool {
        rerail_metal::block_program::ready().is_err()
    }

    /// The compiled grammar's tape is the engine's, node for node (extras and parents too), on
    /// documents it parses; the ones it leaves to the engine come back from there.
    /// Tier 2's grammar calls are the machine's (`by_rules`): nested past `DEPTH`, a file is
    /// refused (to the fallback), not run off the stack. Nested within it, it parses.
    #[test]
    fn deep_nesting_is_refused_not_overflowed() {
        let nested = |n: usize| format!(".decl r(x: number)\nr(x) :- x = {}1{}.\n", "(".repeat(n), ")".repeat(n));
        let c = crate::compiled().expect("compiles");
        let shallow = nested(100);
        assert_eq!(c.parser.image(shallow.as_bytes()).ok(), Some(crate::engine_tape(&shallow)), "100 deep parses as the engine's");
        let deep = nested(3_000);
        assert!(c.parser.image(deep.as_bytes()).is_err(), "3,000 deep: refused");
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
        // A third are still jumps (`GOTO`). A rule file's tests (step 22) raised it from 371 states and 8,860 words.
        assert!(c.program.switches.iter().any(|s| !s.words.is_empty()), "the switches name keywords");
        // Its words in order, by the verifier: out of order, refused.
        let mut bad = c.program.clone();
        let named = bad.switches.iter_mut().find(|s| s.words.len() > 1).expect("a switch naming two words");
        named.words.swap(0, 1);
        assert!(stream::validate(&bad).is_err(), "a switch's words out of order");
        let states = c.machine.lines().filter(|l| l.starts_with("state ")).count();
        let arms = c.machine.lines().filter(|l| l.starts_with("arm ")).count();
        // A call of a nested choice with a field is a call where it can be (the field set where it
        // returns): the choice's level switches on its first token itself, as the lean parser's
        // function does, so more states and arms (371 and 3,068 before). Each rule keeps its own
        // code, as the lean parser's methods do (4,841 words when rules shared alike code), and
        // a term's rest (`more`) is a call: fewer states and arms. A callee that begins with a
        // mark or a merge is called too, at its start (375 states, 3,064 arms, 5,385 words before).
        // With a rule file's tests in the grammar (0130, More-ergonomic) on these calls: 413 states,
        // 3,004 arms, 3,773 words (the grammar's tests alone had taken it to 380 and 9,348). With
        // Parser-generators' tier 2 merged (token marks from the records, 0129; tier 1's checks at
        // the token, 0133): 3,932 words, the states and arms the same. The token marks the grammar
        // takes nowhere tested by `TOKEN`, once a token, not by each state (0139): 3,515. A
        // number's 19 digits checked at its token (`CHECK`, 0133) in the arms that take one: 3,563.
        // A float one token, made a float by the mark after it (`relabel _ as float`): 3,322.
        assert!(states <= 413 && arms <= 3004 && c.program.words.len() <= 3322, "{states} states, {arms} arms, {} words", c.program.words.len());
        let native = native_ir::lower(&c.program, 0, 0).expect("lowers");
        let cut = native_cpu_clusters::by_rules(&native);
        assert_eq!((cut.count as usize, cut.calls), (rules.len(), true), "a function a rule");
        assert_eq!(c.parser.tier2.clusters, rules.len(), "as compiled");
        // The calls open counted by the machine's stack, as the lean parser's: no `RETURN` runs
        // with no call open, and no `ACCEPT` within one, so neither is checked.
        assert!(rerail_compiler::native_cpu_stream::calls_on_stack(&native), "the calls open on the stack");
        // A jump from one rule into another, not a call's: refused, into a later rule or an
        // earlier one. A function is entered by calls alone, as the lean parser's methods are.
        use rerail_compiler::program::{GOTO, PUSH, split, word};
        let own_jump = |p: &rerail_compiler::program::Words, from: usize| (from..p.words.len()).find(|&pc| split(p.words[pc]).0 == GOTO && split(p.words[pc - 1]).0 != PUSH).expect("a rule's own jump");
        // Every call enters its callee's function at its one entry, the rule's first place, as
        // a lean parser's method is entered at its top: a flag the call runs with is its
        // `PUSH`'s, a mark its caller's state keeps for it.
        let firsts: std::collections::HashSet<u16> = rules.iter().copied().collect();
        let w = &c.program.words;
        let mid: Vec<usize> = (1..w.len()).filter(|&pc| split(w[pc - 1]).0 == PUSH && split(w[pc]).0 == GOTO && !firsts.contains(&split(w[pc]).2)).collect();
        assert!(mid.is_empty(), "calls into the middle of a function: {mid:?}");
        for (from, into) in [(rules[1], rules[2]), (rules[2], rules[1] + 2)] {
            let mut bad = c.program.clone();
            let goto = own_jump(&bad, usize::from(from));
            bad.words[goto] = word(GOTO, 0, into);
            assert!(stream::validate_rules(&bad).is_err(), "a jump across rules, into {into}");
        }
        // The machine: no rules. Each shape writes the engine's tape on the samples.
        let m = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        assert!(m.program.rules.is_empty());
        for (i, s) in crate::SAMPLES.iter().enumerate() {
            let engine = Some(crate::engine_tape(s));
            assert_eq!(c.parser.image(s.as_bytes()).ok(), engine, "sample {i}, as rules");
            assert_eq!(m.parser.image(s.as_bytes()).ok(), engine, "sample {i}, as one machine");
        }
    }

    #[test]
    fn tape_is_the_engines() {
        assert_eq!(super::compile_error(), None);
        let parsed = [
            "r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n",
            "r(n) :- n = sum min x : { s(x) } : { t(n) }, m = count : { u(_) }.",
            ".policy {\"a\": [1, {\"b\": null}]}\n/* d */ r(x) :- x ~ `a`, y = \"s\".",
            // A rule file's tests (decision 0130): a fixture set, and a test that uses it.
            ".fixtures app\n    .fixture \"a.py\" ```\n        x = 1\n        ```\n.end\n.test t exactly uses app, b\n    #include \"c.rerail\"\n    seen(1).\n    r(x) :- seen(x).\n    .given seen\n    .expect B101 at \"a.py\":1\n    .expect no r(_)\n.end\nr(x) :- s(x).\n",
            // `exactly` and `uses` as names; a block's statements end at `. end`.
            ".test exactly\n.end\n.test uses uses a\n. // c\n end\n.fixtures f exactly(1).\n.end",
        ];
        for src in parsed {
            assert!(super::compiled_tape(src).is_some(), "the grammar refuses {src:?}");
            assert_eq!(super::tape(src), super::engine_tape(src), "{src:?}");
            // The lean parser writes the same rows.
        }
        // 127 deep, the most (tier 2's count, `COUNT`), is taken; 128 deep is not.
        let most = format!(".policy {{\"a\": {}{}}}", "[".repeat(126), "]".repeat(126));
        assert!(super::compiled_tape(&most).is_some(), "127 deep is taken");
        assert_eq!(super::tape(&most), super::engine_tape(&most));
        let deep = format!(".policy {{\"a\": {}{}}}", "[".repeat(127), "]".repeat(127));
        let refused = [
            "r(x) :- x ~ ````a````.",
            deep.as_str(),
            "r(x) :- .",
            // Blocks do not nest; a `.end` outside one; a block with no `.end`.
            ".test a\n.test b\n.end\n.end",
            ".fixtures a\n.test b\n.end\n.end",
            ".end",
            "r(x).\n.end",
            ".test a\nr(x).",
        ];
        for src in refused {
            assert!(super::compiled_tape(src).is_none(), "the grammar takes {src:?}");
            assert_eq!(super::tape(src), super::engine_tape(src), "{src:?}");
        }
    }

    /// The rule files that carry tests (decision 0130): the grammar takes each, and its tape is the engine's.
    #[test]
    fn rule_files_with_tests_are_the_engines() {
        let machine = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        for src in [
            include_str!("../../../../docs/rule-syntax/packs/python/bandit-b101-assert-used.rerail"),
            include_str!("../../../../docs/rule-syntax/spikes/rr-tests/print-calls.rerail"),
        ] {
            assert!(src.contains(".test "));
            let engine = super::engine_tape(src);
            assert!(super::compiled_tape(src).is_some(), "the grammar refuses a rule file with tests");
            assert_eq!(super::tape(src), engine);
            assert_eq!(machine.parser.image(src.as_bytes()).ok(), Some(engine), "as one machine");
        }
    }

    /// Tier 2 writes the table as it stands at the end (`rust_stream::node_words`): a wrap
    /// placed at once, before the node it wraps (a leaf's swap, or a larger node's rows moved,
    /// their parents with them); a dropped node taken out where it is dropped. Chains of wraps,
    /// wraps over large operands and inside them, and comments among them, as the engine's.
    /// A reread that meets a dropped node's place refuses, as the interpreter, which keeps the
    /// dropped row there, and the reference do.
    #[test]
    fn the_table_is_written_in_place() {
        let parsed = [
            "r(x) :- x = a - b - c - d.",
            "r(x) :- x = (a + b) * (c - d) - e / f.",
            "r(x) :- x = a /* c */ - b // d\n - c.",
            "r(x) :- x = f(a - b, c) - g(d) + 1, y <= x + 2.",
            "r(n) :- n = sum min x : { s(x) } : { t(n) }, m = count : { u(_) }.",
        ];
        for src in parsed {
            assert!(super::compiled_tape(src).is_some(), "the grammar refuses {src:?}");
            assert_eq!(super::tape(src), super::engine_tape(src), "{src:?}");
        }
        let src = "r(n) :- n = min (a, b) : { s(a, b) }.";
        assert!(super::compiled_tape(src).is_none(), "the grammar takes {src:?}");
        assert_eq!(super::tape(src), super::engine_tape(src), "{src:?}");
    }

    /// Tier 2's state machine is built once per rule, not once per place a rule is used: rules
    /// whose calls save states are calls (`grammar::shared`). With every rule inlined into each
    /// place, `.rerail` had 858 states and 8,370 arms; shared, 438 and 4,014. A ratchet: lower it as
    /// the compiler improves. Named-only tapes (step 21) raised it from 234 and 2,225 (to 266 and 2,590): a group's
    /// body and an aggregate word's arguments are rules of their own, which drop their `,`.
    /// A rule file's tests (step 22, decision 0130) raised it to 271 and 2,621: a block's statements are a level of
    /// their own, which shares the setting with the root.
    /// Of the state machine (`Shape::Machine`); the product's shape, a function a rule, has its
    /// own (`tier2_is_a_function_a_grammar_rule`).
    #[test]
    fn the_state_machine_shares_its_rules() {
        let c = super::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles");
        let states = c.machine.lines().filter(|l| l.starts_with("state ")).count();
        let arms = c.machine.lines().filter(|l| l.starts_with("arm ")).count();
        assert!(states <= 271, "{states} states");
        assert!(arms <= 2621, "{arms} arms");
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
            ("../../compiler/src/native_cpu_stream.rs", "pub fn image(&self, bytes: &[u8])"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn image(mut self)"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn tape(&self, batch"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn tape(&self, n: usize)"),
            ("../../compiler/src/native_cpu_stream.rs", "pub fn words(&self, batch"),
            ("../../compiler/src/native_cpu_stream.rs", "fn stream<'b>(&self"),
            ("../../compiler/src/native_cpu_stream.rs", "pub unsafe fn call(&self, bytes"),
            ("../../metal/src/tier2_on.rs", "pub fn image_on(parser: &Parser"),
            ("../../compiler/src/native_cpu_stream.rs", "unsafe fn call_once(&self"),
            ("../../compiler/src/native_cpu_stream.rs", "fn full_tape(&self"),
            ("../../compiler/src/native_cpu_stream.rs", "fn sized(&self"),
            ("lib.rs", "pub fn tapes("),
            ("lib.rs", "fn metal_tapes("),
            ("../../compiler/src/native_cpu_block.rs", "pub fn new(program: &CompiledBlock, files"),
            ("../../compiler/src/native_cpu_block.rs", "pub fn call(&mut self, program: &CompiledBlock)"),
            ("../../compiler/src/native_cpu_block.rs", "fn run(&mut self, entry: Entry)"),
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
    /// `back_fill`s, its position planes) equals Cranelift's over the corpus as one batch: the
    /// records (every position, every code), every total, and every output word but the last-byte
    /// mask the CPU does not make (its lasts are by the end rules, decision 0177: the contract
    /// between the tiers is the records). Prints the GPU time.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
        // The CPU's form leaves out the masks its record pass makes, and the rows of the totals
        // no one reads by them (`BlockNative::cpu_unstored`).
        let unmade = native.cpu_unstored();
        let same_outputs = |g: &[Vec<u64>], w: &[Vec<u64>]| g.len() == w.len() && (0..w.len()).all(|o| unmade.contains(&o) || g[o] == w[o]);
        // Metal sets the gap marks the CPU's tier 2 tests itself (`lazy_marks`).
        let lazy = lazy_bits(native);
        for (k, (g, w)) in got.run.files.iter().zip(&want).enumerate() {
            // A refused file: both refuse it, and its records are the same up to its first
            // refusal (decision 0177, "A refusal leaves the loop"): the CPU's are cut at the
            // token that refused, Metal's at its first token refused, or none where a total
            // refused it, and Metal runs no segment again past a total's (`dropped`), so only
            // the records before the first total's bit (Metal's outputs) are compared.
            let refused = refusal(&w.totals);
            let differs = if refused {
                let cut = native.totals.iter().filter_map(|&t| g.outputs.get(t)).filter_map(|o| o.iter().position(|&x| x != 0).map(|j| j as u32 * 64 + o[j].trailing_zeros())).min().unwrap_or(u32::MAX);
                let before = |v: &[Vec<u32>]| v.iter().map(|p| p.iter().copied().filter(|&x| x < cut).collect::<Vec<_>>()).collect::<Vec<_>>();
                let r = native.records.as_ref().expect("records");
                let n = w.positions[usize::from(r.lasts)].iter().filter(|&&x| x < cut).count().min(w.codes.len()).min(g.codes.len());
                refusal(&g.totals) != refused || !prefix_of(&before(&w.positions), &before(&g.positions)) || unlazy(&g.codes[..n], lazy) != unlazy(&w.codes[..n], lazy)
            } else {
                !same_outputs(&g.outputs, &w.outputs) || g.positions != w.positions || g.totals != w.totals || unlazy(&g.codes, lazy) != unlazy(&w.codes, lazy)
            };
            if differs {
                differ += 1;
                if differ < 4 {
                    let o = (0..w.outputs.len()).find(|&o| !unmade.contains(&o) && g.outputs.get(o) != w.outputs.get(o));
                    eprintln!("file {k} ({} bytes) differs: output {o:?}, positions equal {}, totals {:?} / {:?}", refs[k].len(), g.positions == w.positions, g.totals, w.totals);
                }
            }
        }
        let bytes: usize = refs.iter().map(|b| b.len()).sum();
        eprintln!("{} files, {bytes} bytes: {differ} differ; GPU {:.2} ms ({:.2} GB/s), {} launches, {} segments, {} reruns ({} dropped: refused)", files.len(), got.run.seconds * 1e3, bytes as f64 / got.run.seconds / 1e9, got.run.launches, got.segments, got.reruns, got.dropped);
        assert_eq!(differ, 0);
    }

    /// A file refused at its start, then many tokens: Metal's record pass stops at the first
    /// token refused (decision 0177, "A refusal leaves the loop"), as the CPU's loop does. Its
    /// codes end there and are the CPU's; the same tokens without the refusal are all coded.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_refused_files_records_stop_on_metal() {
        if no_device() {
            return;
        }
        use rerail_compiler::block;
        use rerail_metal::block_program::serial::{Segments, run_serial_split};
        let c = super::compiled().expect("compiles");
        let lazy = lazy_bits(&c.native);
        let body = "r(x) :- a(x), \"s\". // c\n".repeat(400);
        // Refused by a mask total (no token coded), and by a token the record pass refuses: an
        // integer of 19 digits, the engine's (decision 0413; a float chain is read now).
        for head in ["\u{1}", "````a````.", "x = 1234567890123456789."] {
            let (bad, good) = (format!("a(x).\n{head}\n{body}"), format!("a(x).\n{body}"));
            let refs = [bad.as_bytes(), good.as_bytes()];
            let files: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
            let got = run_serial_split(&[&c.native], &files, Segments { blocks: 29, warm: 1 }).expect("Metal runs tier 1");
            let want = c.parser.tier1.run_files(&refs);
            let (g, w) = (&got.run.files[0], &want[0]);
            assert!(refusal(&g.totals) && refusal(&w.totals), "{head:?}: refused on both");
            assert!(g.codes.len() < 16, "{head:?}: Metal's codes stop at the refusal: {} of {}", g.codes.len(), g.positions[0].len());
            eprintln!("{head:?}: Metal coded {} tokens, the CPU {}", g.codes.len(), w.codes.len());
            let n = g.codes.len().min(w.codes.len());
            assert_eq!(unlazy(&g.codes[..n], lazy), unlazy(&w.codes[..n], lazy), "{head:?}: the codes before the refusal");
            let (g, w) = (&got.run.files[1], &want[1]);
            assert!(!refusal(&g.totals), "accepted");
            assert_eq!(unlazy(&g.codes, lazy), unlazy(&w.codes, lazy), "the accepted file's codes, all of them");
            assert!(g.codes.len() > 2000);
        }
        // Strings over many segments, refused in the first: with the scans' carries speculated
        // (`RERAIL_SERIAL_CONE=0`) the later segments' starts are wrong and listed, and none of
        // them is run again (`dropped`): the total is in a segment before them, whose start is
        // final. With the carries exact (the default) none is listed.
        let long = format!("a(\"{}\").\n", "s ".repeat(3000)).repeat(4);
        let (bad, good) = (format!("\u{1}\n{long}"), long.clone());
        for (src, refused) in [(&bad, true), (&good, false)] {
            let f = [block::BatchFile { bytes: src.as_bytes(), program: 0 }];
            let got = run_serial_split(&[&c.native], &f, Segments { blocks: 29, warm: 1 }).expect("Metal runs tier 1");
            let w = &c.parser.tier1.run_files(&[src.as_bytes()])[0];
            assert_eq!(refusal(&got.run.files[0].totals), refused);
            if !refused {
                assert_eq!(unlazy(&got.run.files[0].codes, lazy), unlazy(&w.codes, lazy));
            }
            eprintln!("refused {refused}: {} segments, {} run again, {} dropped", got.segments, got.reruns - got.dropped, got.dropped);
            if std::env::var("RERAIL_SERIAL_CONE").is_ok_and(|v| v == "0") {
                assert!(got.reruns > 0, "the speculated starts are wrong");
                assert_eq!(got.dropped > 0, refused, "a refused file's segments are not run again");
            }
        }
    }

    /// `tapes` (tier 1 on Metal, tier 2 over its buffers in place) gives `tape`'s tables: on
    /// small documents, the ones the grammar refuses among them.
    #[test]
    fn tapes_are_tape() {
        let deep = format!(".policy {{\"a\": {}{}}}", "[".repeat(127), "]".repeat(127));
        let long = "r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n".repeat(200);
        let files = ["r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n", "r(x) :- ````a````.", ".policy {\"a\": [1, {\"b\": null}]}\n/* d */ r(x) :- x ~ `a`, y = \"s\".", deep.as_str(), "", long.as_str()];
        let got = super::tapes(&files);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let mut on = Vec::new();
            assert!(no_device() || super::metal_tapes(super::compiled().expect("compiles"), &files, &mut on), "Metal runs the batch");
            assert!(no_device() || on == got, "Metal's tapes, the CPU's");
        }
        for (src, g) in files.iter().zip(&got) {
            assert_eq!(*g, super::tape(src), "{src:?}");
        }
    }

    /// Tier 1's one pass on the CPU, split by what it makes: compiled as it is, without the
    /// record pass, and without the extracts too, each run over the corpus as one batch; the
    /// instructions retired, the least of five. A probe: run alone, one thread.
    #[test]
    #[ignore = "a probe: run by hand, --test-threads=1"]
    fn rr_tier1_split() {
        use rerail_compiler::{native_cpu_block, native_cpu_codegen::retired};
        let c = crate::compiled().expect("compiles");
        let corpus = corpus();
        let files: Vec<&[u8]> = corpus.iter().map(|f| f.as_bytes()).collect();
        let full = c.native.clone();
        let mut no_records = full.clone();
        no_records.records = None;
        let mut no_extracts = no_records.clone();
        no_extracts.extracts.clear();
        if let Some(r) = &full.records {
            let alone = r.marks.iter().filter(|m| m.0 == rerail_compiler::records::WORD_ALONE).count();
            eprintln!("records: {} marks ({alone} of a word alone), {} words; the marks' outputs {:?}", r.marks.len(), r.words.len(), r.marks);
            let labels: Vec<&str> = r.marks.iter().filter(|m| m.0 != rerail_compiler::records::WORD_ALONE).map(|m| c.labels.iter().find(|l| l.1 == m.0).map_or("?", |l| l.0.as_str())).collect();
            eprintln!("  masked marks: {labels:?}");
        }
        let tokens: usize = files.iter().map(|f| c.parser.tier1.extract(f).codes.len()).sum();
        eprintln!("{tokens} tokens");
        let mut last = 0u64;
        // Then no totals (the refusals' flags), and no output at all: what is left is the regions'
        // walk and the loop over the blocks.
        let mut no_totals = no_extracts.clone();
        no_totals.totals.clear();
        let mut nothing = no_totals.clone();
        nothing.outputs.clear();
        nothing.output_delays.clear();
        nothing.masks.clear();
        for (name, form) in [("as it is", &full), ("no record pass", &no_records), ("no extracts either", &no_extracts), ("no totals either", &no_totals), ("no output at all", &nothing)] {
            let compiled = match native_cpu_block::compile(form) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{name:>20}: does not compile: {e:?}");
                    continue;
                }
            };
            let mut batch = native_cpu_block::Batch::new(&compiled, &files);
            let best = (0..5)
                .map(|_| {
                    let a = retired().0;
                    batch.call(&compiled);
                    retired().0 - a
                })
                .min()
                .expect("rounds");
            let bytes: usize = files.iter().map(|f| f.len()).sum();
            eprintln!(
                "{name:>20}: {:7.1}M instructions, {:5.1} a byte, {:5.1} a token{}",
                best as f64 / 1e6,
                best as f64 / bytes as f64,
                best as f64 / tokens as f64,
                if last > 0 { format!(" ({:+.1}M)", (best as f64 - last as f64) / 1e6) } else { String::new() }
            );
            last = best;
        }
    }

    /// Tier 2's words as text, to `RR_WORDS` (a probe of what the marks cost).
    #[test]
    #[ignore = "a probe: run by hand with RR_WORDS"]
    fn rr_words_text() {
        let c = crate::compiled().expect("compiles");
        std::fs::write(std::env::var("RR_WORDS").expect("set RR_WORDS"), format!("{}\n---\n{}", c.words, c.machine)).expect("writes");
    }

    /// The grammar's regions walked (the CPU's form, decision 0139) are its rounds' fixed point
    /// (Metal's): every value of tier 1 the same, on a sample of the corpus and on regions whose
    /// closers cross a block's end, and empty literals with and without a `` `` `` later.
    #[test]
    fn the_walks_are_the_rounds() {
        let c = crate::compiled().expect("compiles");
        assert_eq!(c.block.walks.len(), 1, "the grammar's regions are one walk");
        let pad = |n: usize| "x".repeat(n);
        let edges = [
            format!("{}```a```", pad(55)),
            format!("{}```a``` r(x).", pad(58)),
            format!("{}\"{}\" r(x).", pad(40), "s".repeat(100)),
            format!("{}/* {} */ r(x).", pad(61), "c".repeat(70)),
            format!("{}// c\n r(x).", pad(62)),
            format!("r(x) :- x = ``, y = ``.{}", pad(90)),
            format!("r(x) :- x = ``, y = ``a``.{}``", pad(90)),
            format!("{}`` {} ``", pad(30), pad(200)),
            "r(x) :- `a\"b`, \"c`d\", `` `` ``.".to_owned(),
            "\"open at the end".to_owned(),
            "/* open at the end".to_owned(),
            "``".to_owned(),
        ];
        let corpus = corpus();
        let mut files: Vec<&[u8]> = corpus.iter().step_by(10).map(|f| f.as_bytes()).collect();
        files.extend(edges.iter().map(|f| f.as_bytes()));
        let (walked, spans) = c.block.all_values(&files);
        let (rounds, _) = c.block.rounds_values(&files);
        for (v, (w, r)) in walked.iter().zip(&rounds).enumerate() {
            if w != r {
                let b = w.iter().zip(r).position(|(x, y)| x != y).expect("a word differs");
                let f = spans.iter().position(|s| s.contains(&b)).expect("a file's");
                panic!("value {v} differs in file {f} block {}: walked {:#x}, rounds {:#x}", b - spans[f].start, w[b], r[b]);
            }
        }
        // And the CPU's tier 1 (the walk lowered) gives the engine's tapes on the edges.
        for f in &edges {
            if let Ok(t) = c.parser.image(f.as_bytes()) {
                assert_eq!(t, crate::engine_tape(f), "{f:?}");
            }
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
            let engine = crate::engine_tape(f);
            for (k, c) in [rules, &machine].into_iter().enumerate() {
                if let Ok(t) = c.parser.image(f.as_bytes()) {
                    taken[k] += 1;
                    // The engine's, and the lean parser's rows byte for byte.
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

    /// The padding is the compiler's input (`BlockNative::pad`): the grammar compiled to count on
    /// none writes the default's tapes, and takes the same files (its windows cut on the rare
    /// branch: `native_cpu_bytes`' `a_padded_windows_word_is_not_cut`).
    #[test]
    #[ignore = "the corpus: run by name"]
    fn an_unpadded_programs_tapes_are_the_padded() {
        let padded = crate::compiled().expect("compiles");
        let bare = crate::compile_padded(crate::GRAMMAR, rerail_compiler::bitlex_masks::Shape::Rules, 0).expect("compiles unpadded");
        assert_eq!((padded.parser.tier1.pad, bare.parser.tier1.pad), (rerail_compiler::native_cpu_block::BACK, 0));
        let mut differ = Vec::new();
        for (i, f) in corpus().iter().enumerate() {
            if padded.parser.image(f.as_bytes()).ok() != bare.parser.image(f.as_bytes()).ok() {
                differ.push(i);
            }
        }
        assert_eq!(differ, Vec::<usize>::new(), "files whose tapes differ");
    }

    /// The engine's bytes over the corpus, against the lean parser's: the interpreter's tape
    /// (`stream::image`), and the JIT's, as its generated code writes it (decision 0140,
    /// docs/parser-generators/37).
    #[test]
    #[ignore = "the corpus: run by name"]
    fn corpus_images_are_the_lean_tapes() {
        use rerail_tape_format::RR_TAG;
        let files = corpus();
        let c = crate::compiled().expect("compiles");
        let extras = &c.program.readings.extras;
        let (mut taken, mut jit, mut interp) = (0, Vec::new(), Vec::new());
        for (i, f) in files.iter().enumerate() {
            let Ok(image) = c.parser.image(f.as_bytes()) else { continue };
            taken += 1;
            let lean = crate::rr::lean::tape(f);
            if image != lean {
                jit.push(i);
            }
            let run = c.program.block_programs[0].run(f.as_bytes());
            let same = rerail_compiler::stream::run(&c.program, f.as_bytes(), &run).is_ok_and(|n| rerail_compiler::stream::image(&n, extras, RR_TAG, f.as_bytes()) == lean);
            if !same {
                interp.push(i);
            }
        }
        eprintln!("{} files, {taken} taken", files.len());
        assert_eq!(jit, Vec::<usize>::new(), "files whose JIT image is not the lean tape");
        assert_eq!(interp, Vec::<usize>::new(), "files whose interpreter image is not the lean tape");
    }

    /// The gap marks tier 2 on the CPU tests itself (`native_cpu_block::lazy_marks`: the line
    /// marks, where a statement asks whether the next token is on its line) read the token's
    /// `Next` and `Start` as their caller has them: in one function, whose state is in a stack
    /// slot, as in clusters by rules, whose state is in `ctx`. Each tree is the interpreter's: a
    /// `.policy` with no object ends at its line, a setting's arguments are on its line.
    /// PR #508's 0d1f9497e (t47).
    #[test]
    fn a_lazy_marks_token_is_its_callers() {
        use rerail_compiler::native_cpu_clusters as cl;
        let c = crate::compiled().expect("compiles");
        let mut native = rerail_compiler::native_ir::lower(&c.program, 0, 0).expect("lowers");
        native.tag = rerail_tape_format::RR_TAG;
        let n = native.structural.len();
        for (shape, cut) in [("one function", cl::Cut::one(n)), ("by rules", cl::by_rules(&native))] {
            let parser = rerail_compiler::native_cpu_stream::Parser::with_cut(&native, &cut).expect("compiles");
            for src in [".policy\nmust().", ".mode\nr(x).", ".policy // c\n\nmust().", ".mode a\n.policy must().", ".policy must()."] {
                let run = c.program.block_programs[0].run(src.as_bytes());
                let want = rerail_compiler::stream::run(&c.program, src.as_bytes(), &run).map(rerail_compiler::ladder::as_ids).expect("the interpreter parses it");
                let got = parser.parse(src.as_bytes()).map(rerail_compiler::ladder::as_ids).ok();
                assert_eq!(got.as_ref(), Some(&want), "{shape}: {src:?}");
            }
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
        let pushes: Vec<usize> = words.iter().filter(|&&w| split(w).0 == PUSH).map(|&w| usize::from(split(w).2)).collect();
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
                    if let Some(&j) = index.get(&n)
                        && j != i {
                            *fixed.entry((i.min(j), i.max(j))).or_default() += 1;
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
            order.sort_by_key(|b| std::cmp::Reverse(b.1));
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
                cand.sort_by_key(|b| std::cmp::Reverse(b.1));
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
        rerail_prof::process_cpu()
    }

    /// Where a Metal run's wall time goes: two runs in a row over the corpus's first files.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
        // The CPU's form (its walks walked, decision 0139), then Metal's (their rounds run).
        let cpu = c.native.cpu_form();
        let cpu_passes = cpu.passes();
        eprintln!("the CPU's form: {} passes", cpu_passes.passes.len());
        for (k, pass) in cpu_passes.passes.iter().enumerate() {
            if let Pass::Forward { body, stores } = pass {
                let count = |f: &dyn Fn(&BlockInst) -> bool| body.insts.iter().filter(|i| f(i)).count();
                eprintln!("  pass {k}: forward, {} insts, {} walk values, {} fills, {} prevs, {} groups, {} outputs, {} stores", body.insts.len(), count(&|i| matches!(i, BlockInst::Walk { .. })), count(&|i| matches!(i, BlockInst::Fill(..))), count(&|i| matches!(i, BlockInst::Prev(..))), body.groups.len(), body.outputs.len(), stores.len());
            } else {
                eprintln!("  pass {k}: back");
            }
        }
        eprintln!("Metal's form:");
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

    /// Tier 1 of `.rerail` on Metal, its kernels made first: `prewarm` over a sample (`RR_SAMPLE`)
    /// runs the program, learns, and compiles the learned program's kernels, as a
    /// process would off the parse's path. The timed runs then must compile nothing
    /// (`compiles`). Each run: the corpus (`RR_REPEAT` times) as batches under the device's
    /// budget, halved where refused; its GPU seconds by kind (the first pass, the runs again,
    /// the positions), its launches and rounds, and the host's CPU and wall.
    #[cfg(target_os = "macos")]
    #[test]
    #[cfg(target_os = "macos")]
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

    /// Tier 2 in clusters (`native_cpu_clusters`, doc 28) on `.rerail`: the traffic from every
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
        let nodes: usize = ours.iter().map(|f| super::rr::engine_tape(f).len()).sum();
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
                n += super::rr::engine_tape(f).len();
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
        for (pc, &r) in runs.iter().enumerate() {
            let op = format!("{:?}", native.structural[pc].operation);
            let e = by.entry(op.split([' ', '{', '(']).next().unwrap_or("").to_owned()).or_default();
            e.0 += r;
            e.1 += r * u64::from(insts.get(pc).copied().unwrap_or(0));
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.1.1));
        let total: u64 = v.iter().map(|x| x.1.1).sum();
        let pushes = rerail_compiler::native_cpu_stream::push_targets(&native);
        // The switches as run: their runs of bytes, weighted by how often each switch runs.
        let (mut sw_runs, mut sw_n) = (0u64, 0u64);
        for (pc, &r) in runs.iter().enumerate() {
            if let rerail_compiler::native_ir::StructuralOperation::Switch { table } = native.structural[pc].operation {
                let t = &native.switches[usize::from(table)].targets;
                let n = 1 + t.windows(2).filter(|w| w[0] != w[1]).count() as u64;
                sw_runs += r * n;
                sw_n += r;
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
    /// Where tier 2's work goes, by opcode: each word's runs over the corpus (the interpreter's,
    /// `run_seen`) times its machine code in its rule's function (`word_bytes`, every path of
    /// it: an upper bound), and the runs alone. The functions the words call are not in it.
    #[test]
    #[ignore = "a measurement: run by hand"]
    fn tier2_work_by_opcode() {
        use rerail_compiler::program::split;
        let c = crate::compiled().expect("compiles");
        let program = &c.program;
        let bytes = &c.parser.tier2.word_bytes;
        let mut runs = vec![0u64; program.words.len()];
        let mut tokens = 0u64;
        for src in corpus() {
            let run = program.block_programs[0].run(src.as_bytes());
            tokens += run.codes.len() as u64;
            let _ = rerail_compiler::stream::run_seen(program, src.as_bytes(), &run, &mut |pc| runs[pc] += 1);
        }
        let mut by: std::collections::BTreeMap<(u8, u8), (u64, u64)> = Default::default();
        for (pc, &n) in runs.iter().enumerate() {
            let (op, mode, _) = split(program.words[pc]);
            let e = by.entry((op, if op == rerail_compiler::program::MARK { 0 } else { mode })).or_default();
            e.0 += n;
            e.1 += n * u64::from(bytes.get(pc).copied().unwrap_or(0) / 4);
        }
        // The switches run, by their ways: distinct targets but the refusal and the end.
        let mut ways: std::collections::BTreeMap<usize, (u64, u64)> = Default::default();
        for (pc, &n) in runs.iter().enumerate() {
            let (op, _, t) = split(program.words[pc]);
            if op != rerail_compiler::program::SWITCH {
                continue;
            }
            let s = &program.switches[usize::from(t)];
            let mut d: Vec<u16> = s.targets.iter().chain(s.words.iter().map(|w| &w.1)).copied().filter(|&x| x != rerail_compiler::stream_ir::REFUSE && x != s.end).collect();
            d.sort_unstable();
            d.dedup();
            let e = ways.entry(d.len()).or_default();
            e.0 += n;
            e.1 += n * u64::from(bytes.get(pc).copied().unwrap_or(0) / 4);
        }
        // The switches run most: each kind's target (by the records' kinds), and the words'.
        let kinds = &program.block_programs[0].records.as_ref().expect("records").kinds;
        let mut top: Vec<(u64, usize)> = runs.iter().enumerate().filter(|&(pc, _)| split(program.words[pc]).0 == rerail_compiler::program::SWITCH).map(|(pc, &n)| (n, pc)).collect();
        top.sort_unstable_by(|a, b| b.cmp(a));
        for &(n, pc) in top.iter().take(6) {
            let s = &program.switches[usize::from(split(program.words[pc]).2)];
            let mut by_kind: std::collections::BTreeMap<u8, u16> = Default::default();
            for (b, &t) in s.targets.iter().enumerate() {
                by_kind.insert(kinds[b], t);
            }
            let show = |t: u16| if t == rerail_compiler::stream_ir::REFUSE { "R".to_string() } else if t == s.end { "E".to_string() } else { format!("@{t}") };
            let line: Vec<String> = by_kind.iter().map(|(k, &t)| format!("{k}:{}", show(t))).collect();
            let words: Vec<String> = s.words.iter().map(|&(w, t)| format!("s{w}:{}", show(t))).collect();
            eprintln!("  switch @{pc} runs {:.2} a token, {} bytes: {} | words {}", n as f64 / tokens as f64, bytes[pc], line.join(" "), words.join(" "));
        }
        for (w, (n, ins)) in &ways {
            eprintln!("  switches of {w} ways: runs {:.2} a token, at most {:.1} instructions a run", *n as f64 / tokens as f64, *ins as f64 / (*n).max(1) as f64);
        }
        let mut rows: Vec<_> = by.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1.1));
        eprintln!("{tokens} tokens");
        for ((op, mode), (n, ins)) in rows.iter().take(25) {
            eprintln!("  op {op:#04x} m={mode:<3} runs {:>6.2} a token, instructions (at most) {:>6.1} a token", *n as f64 / tokens as f64, *ins as f64 / tokens as f64);
        }
    }

    /// A token mark (decision 0129), `no_value`: an aggregate's word, then `(`, `)`, `:`, with
    /// comments between or none, is refused by every shape and the interpreter; with a value,
    /// taken, as the engine's tape.
    #[test]
    fn token_marks_refuse_and_take() {
        let c = crate::compiled().expect("compiles");
        // Each token mark's bit in the record is its first word's alone: no mask read.
        let alone = c.block.records.as_ref().expect("records").marks.iter().filter(|m| m.0 == rerail_compiler::records::WORD_ALONE).count();
        assert_eq!(alone, c.program.token_marks.len(), "the token marks' bits by their words");
        let machine = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        let head = ".decl a(n: number, j: number)\n.decl d(n: number, k: number)\n";
        let rule = |agg: &str| format!("{head}d(n, k) :- a(n, _), k = {agg} : {{ a(n, j) }}.\n");
        for (agg, taken) in [("max j", true), ("max (j)", true), ("max ()", false), ("min /* c */ ( /* d */ )", false), ("join ()", false), ("count", true)] {
            let src = rule(agg);
            let interp = rerail_compiler::stream::run(&c.program, src.as_bytes(), &c.program.block_programs[0].run(src.as_bytes()));
            assert_eq!(interp.is_ok(), taken, "{agg}: the interpreter");
            for p in [&c.parser, &machine.parser] {
                match p.image(src.as_bytes()) {
                    Ok(t) => {
                        assert!(taken, "{agg}: taken");
                        assert_eq!(t, crate::engine_tape(&src), "{agg}: the engine's tape");
                    }
                    Err(_) => assert!(!taken, "{agg}: refused"),
                }
            }
        }
    }

    /// The token marks the grammar takes nowhere (`TokenMark::refuses`): `TOKEN` tests them, once
    /// a token, and no state does. A functor named, not called, before a token that can not
    /// follow it; an I/O directive with no relation before an `#include` or at the end: refused
    /// by every shape and the interpreter, comments between the tokens or none; else taken, as
    /// the engine's tape.
    #[test]
    fn refusing_token_marks_refuse_at_any_level() {
        let c = crate::compiled().expect("compiles");
        let refusing = c.program.token_marks.iter().filter(|m| m.refuses).count();
        assert_eq!(refusing, 3, "`no_value`, `fn_uncalled`, `io_bare`");
        let words = c.program.words.iter().filter(|&&w| rerail_compiler::program::split(w).0 == rerail_compiler::program::MARK);
        assert!(words.clone().all(|&w| !c.program.token_marks.iter().any(|m| m.refuses && m.order == rerail_compiler::program::split(w).1 % 32)), "no state tests a refusing mark");
        let machine = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        let head = ".decl a(x: number)\n";
        let cases = [
            (format!("{head}a(x) :- a(y), x = @f(y).\n"), true),
            (format!("{head}a(x) :- a(y), x = @f /* c */ (y).\n"), true),
            (format!("{head}a(x) :- a(y), x = @f.\n"), false),
            (format!("{head}a(x) :- a(y), x = @f /* c */ .\n"), false),
            (format!("{head}a(x) :- a(y), x = @f + 1.\n"), false),
            (format!("{head}.input a\n"), true),
            (format!("{head}.output a, a\n.input a\n"), true),
            (format!("{head}.input\n.decl b(x: number)\n"), true),
            (format!("{head}.input\n"), false),
            (format!("{head}.output /* c */\n"), false),
            (format!("{head}.input\n#include \"x.rerail\"\n"), false),
        ];
        for (src, taken) in &cases {
            let interp = rerail_compiler::stream::run(&c.program, src.as_bytes(), &c.program.block_programs[0].run(src.as_bytes()));
            assert_eq!(interp.is_ok(), *taken, "{src}: the interpreter {:?}", interp.as_ref().err());
            for p in [&c.parser, &machine.parser] {
                match p.image(src.as_bytes()) {
                    Ok(t) => {
                        assert!(*taken, "{src}: taken");
                        assert_eq!(t, crate::engine_tape(src), "{src}: the engine's tape");
                    }
                    Err(_) => assert!(!*taken, "{src}: refused"),
                }
            }
        }
    }

    /// An integer of 19 digits or more goes to the engine, checked at its token (decision 0133's
    /// local check: the token's first 19 bytes); 18 digits are taken, as the engine's tape, and
    /// a float of any length. In
    /// a block or across two, by every shape and the interpreter.
    #[test]
    fn long_numbers_go_to_the_engine() {
        let c = crate::compiled().expect("compiles");
        assert!(c.values.iter().all(|v| !["sig", "sig16", "long_int"].contains(&v.name.as_str())), "no mask counts digits");
        let machine = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        let d18 = "123456789012345678";
        let d19 = "1234567890123456789";
        let mut cases = Vec::new();
        for pad in [0usize, 40, 50, 60] {
            let head = format!(".decl a(x: number)\n// {}\n", "x".repeat(pad));
            cases.push((format!("{head}a({d18}).\n"), true));
            cases.push((format!("{head}a({d18}.5).\n"), true));
            // A float's digits are its text: no limit.
            cases.push((format!("{head}a({d19}.5).\n"), true));
            cases.push((format!("{head}a(x) :- a(y), x = y + {d18}.\n"), true));
            cases.push((format!("{head}a({d19}).\n"), false));
            cases.push((format!("{head}a({d19}0).\n"), false));
            cases.push((format!("{head}a(x) :- a(y), x = y + {d19}.\n"), false));
        }
        for (src, taken) in &cases {
            let interp = rerail_compiler::stream::run(&c.program, src.as_bytes(), &c.program.block_programs[0].run(src.as_bytes()));
            assert_eq!(interp.is_ok(), *taken, "{src}: the interpreter {:?}", interp.as_ref().err());
            for p in [&c.parser, &machine.parser] {
                match p.image(src.as_bytes()) {
                    Ok(t) => {
                        assert!(*taken, "{src}: taken");
                        assert_eq!(t, crate::engine_tape(src), "{src}: the engine's tape");
                    }
                    Err(_) => assert!(!*taken, "{src}: refused"),
                }
            }
        }
    }

    /// A token's end by its rule (`run`, `pair`, decision 0177) is the last-byte mask it was
    /// written as before: the same positions, every token's, over a corpus sample and the
    /// samples, from the grammar both ways.
    #[test]
    fn token_rules_are_their_last_masks() {
        let old = crate::GRAMMAR
            .replace("start hash & directive run word", "start hash & directive last hashword & ~next(hashword & ~directive)")
            .replace("start alpha & ident & ~prev(ident) run word", "start alpha & ident & ~prev(ident) last ident & ~next(ident)")
            .replace("start digit & int & ~prev(int) run num_run once dot", "start digit & int & ~prev(int) last numeral & ~next(numeral)")
            .replace("start punct & p_start pair two_start", "start punct & p_start last p_last")
            .replace("read str", "last prev(f_str) & ~f_str")
            .replace("read line | block", "last (f_line & ~next(f_line)) | (prev(f_block) & ~f_block)")
            .replace("read t1 | t2 | t3", "last (prev(f_t1) & ~f_t1) | (prev(f_t2, 2) & ~prev(f_t2)) | (prev(f_t3, 3) & ~prev(f_t3, 2)) | prev(e_t2)")
            // The gap pass's clauses need the loop; this compares the starts and lasts alone.
            .replace("lf unless after \",\", end", "lf, end")
            .replace(", else escape \"a float the file ends with\"", "")
            // The marks' token tests need the loop too: their masks alone.
            .replace(", after number when count(dot) at least 1", "")
            .replace("mark json_bad    = ff,", "mark json_bad    = ff")
            .lines()
            .filter(|l| !(l.starts_with("escape ") && l.contains(" at number when ")))
            .filter(|l| !(l.starts_with("                   after ") || l.starts_with("                   at ")))
            .map(|l| format!("{l}\n"))
            .collect::<String>();
        assert!(old.lines().filter(|l| l.starts_with("token ")).all(|l| !l.contains(" run ") && !l.contains(" pair ") && !l.contains(" read ")), "every rule written back as its mask");
        let (a, b) = (crate::compiled().expect("compiles"), crate::compile_text(&old, rerail_compiler::bitlex_masks::Shape::Rules).expect("the old form compiles"));
        let files = corpus();
        let mut inputs: Vec<&[u8]> = files.iter().step_by(7).map(|f| f.as_bytes()).collect();
        inputs.extend(crate::SAMPLES.iter().map(|s| s.as_bytes()));
        // Runs of two-byte spellings, and one at the end: the next byte read, against the mask.
        inputs.extend([":->", "a :-- b.", "x<:y", "a<=:b", "!=>=", "->->", "a <", "b :", "-", "=>=>="].map(str::as_bytes));
        let (ra, rb) = (a.program.block_programs[0].records.as_ref().expect("records"), b.program.block_programs[0].records.as_ref().expect("records"));
        for f in &inputs {
            let (x, y) = (a.program.block_programs[0].run(f), b.program.block_programs[0].run(f));
            assert_eq!(x.positions[usize::from(ra.starts)], y.positions[usize::from(rb.starts)], "the starts");
            let (lx, ly) = (&x.positions[usize::from(ra.lasts)], &y.positions[usize::from(rb.lasts)]);
            if let Some(k) = lx.iter().zip(ly).position(|(p, q)| p != q) {
                let s0 = x.positions[usize::from(ra.starts)][k] as usize;
                let text = String::from_utf8_lossy(&f[s0.saturating_sub(10)..(lx[k].max(ly[k]) as usize + 3).min(f.len())]).into_owned();
                panic!("token {k} at {s0}: the rule's last {}, the mask's {}: {text:?}", lx[k], ly[k]);
            }
            assert_eq!(lx.len(), ly.len(), "the lasts' count");
        }
        eprintln!("{} files: the same starts and lasts", inputs.len());
    }

    /// The end rules' lasts (decision 0177) are the lasts the masks give: every token's, over the
    /// corpus and the samples. The interpreter takes its lasts from the rules; the extract of the
    /// last-byte masks is the meaning they replace.
    #[test]
    fn end_rules_give_the_masks_lasts() {
        let c = crate::compiled().expect("compiles");
        let block = &c.program.block_programs[0];
        let r = block.records.as_ref().expect("records");
        assert!(!r.rules.is_empty(), "the grammar's tokens have end rules");
        let lasts_out = block.extract_outputs()[usize::from(r.lasts)];
        let files = corpus();
        let mut inputs: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        inputs.extend(crate::SAMPLES.iter().map(|s| s.as_bytes()));
        let mut tokens = 0;
        for f in &inputs {
            let run = block.run(f);
            let words = &run.outputs[lasts_out];
            let count = words.iter().map(|w| w.count_ones() as usize).sum();
            let masks = rerail_compiler::block::extract::positions(words, count);
            assert_eq!(run.positions[usize::from(r.lasts)], masks, "the rules' lasts");
            tokens += masks.len();
        }
        eprintln!("{} files, {tokens} tokens: the rules' lasts are the masks'", inputs.len());
    }

    /// The token loop reads back no record it wrote (`PassShape::reads_back`), as the lean
    /// lexer keeps its cursor and its window's `base` in registers: the refill knows the block
    /// it read last from its state, and the gap marks are tier 2's (`lazy_marks`), `json_bad`'s
    /// form feed among them, so no token's gap starts from the record before.
    #[test]
    fn the_loop_reads_no_record_back() {
        let c = crate::compiled().expect("compiles");
        let r = c.native.records.as_ref().expect("records");
        assert!(rerail_compiler::native_cpu_block::lazy_coded(r) != 0, "a gap mark with token tests (`json_bad`)");
        let shape = c.parser.tier1.passes.last().expect("a pass");
        assert_eq!(shape.reads_back, 0, "the token loop reads a record back");
    }

    /// A run rule with a short form (a number: its 16 bytes from one window, its tests from the
    /// same bits) is read in the token loop's arm for its first byte, no call, as the lean
    /// lexer reads a number in its `match` arm (`PassShape::arms`). A longer one calls the
    /// rule's whole function (`a_numbers_checks_are_at_its_token` reads those).
    #[test]
    fn a_short_run_is_the_loops_arm() {
        let c = crate::compiled().expect("compiles");
        let shape = c.parser.tier1.passes.last().expect("a pass");
        assert!(shape.arms >= 1, "no run rule read in its arm");
    }

    /// A rare start's arms each write their own token, as the lean lexer's `match` arms: a
    /// tail an arm (a string's, `#`'s, a comment's, a backtick's, a number's), each knowing its
    /// first bytes' entry where they share one (`PassShape::known_entries`), so its kind and
    /// whether it is a word are constants of its tail, not tests of a loaded entry.
    #[test]
    fn a_rare_arm_knows_its_entry() {
        let c = crate::compiled().expect("compiles");
        let shape = c.parser.tier1.passes.last().expect("a pass");
        assert!(shape.known_entries >= 5, "rare arms with their entry known: {}", shape.known_entries);
    }

    /// A reader with a short form (a comment's, a string's, a backtick's: its openers and its
    /// first window, the long read its whole function's call) is read in the loop's arm too,
    /// as the lean lexer's `b'/'` arm tests the byte after and scans a line comment in place:
    /// the number's arm and the readers' (`PassShape::arms`), no short function between.
    #[test]
    fn a_readers_short_form_is_the_loops_arm() {
        let c = crate::compiled().expect("compiles");
        let shape = c.parser.tier1.passes.last().expect("a pass");
        assert!(shape.arms >= 4, "the run's and the readers' arms: {}", shape.arms);
        // The number's by name: its cut (`run … once dot`) is made in the run's window, so its
        // short form calls nothing but its whole function (decision 0413).
        let r = c.native.records.as_ref().expect("records");
        for (what, b) in [("the number", b'7'), ("the string", b'"'), ("the comment", b'/')] {
            let k = r.ends[usize::from(b)];
            assert!(shape.arm_rules.contains(&k), "{what}'s rule {k} is no arm: {:?}", shape.arm_rules);
        }
    }

    /// Decision 0177, stage 2a: the tokens by the loop, each at the first byte after the one
    /// before that `skip` does not hold, are the starts' and lasts' masks, on every file the
    /// masks take (a file they refuse, its totals' flag set, is the engine's either way).
    #[test]
    fn the_loops_tokens_are_the_masks() {
        let c = crate::compiled().expect("compiles");
        let block = &c.program.block_programs[0];
        let r = block.records.as_ref().expect("records");
        assert!(r.skip.is_some(), "the records have the bytes between tokens");
        let outs = block.extract_outputs();
        let (starts_out, lasts_out) = (outs[usize::from(r.starts)], outs[usize::from(r.lasts)]);
        let files = corpus();
        let mut inputs: Vec<&[u8]> = files.iter().map(|f| f.as_bytes()).collect();
        inputs.extend(crate::SAMPLES.iter().map(|s| s.as_bytes()));
        let (mut tokens, mut refused) = (0, 0);
        for f in &inputs {
            let run = block.run(f);
            if run.totals.iter().any(|&t| t != 0) {
                refused += 1;
                continue;
            }
            let mask = |o: usize| {
                let words = &run.outputs[o];
                rerail_compiler::block::extract::positions(words, words.iter().map(|w| w.count_ones() as usize).sum())
            };
            let starts = mask(starts_out);
            assert_eq!(run.positions[usize::from(r.starts)], starts, "the loop's starts");
            assert_eq!(run.positions[usize::from(r.lasts)], mask(lasts_out), "the loop's lasts");
            tokens += starts.len();
        }
        eprintln!("{} files ({refused} refused), {tokens} tokens: the loop's are the masks'", inputs.len());
    }

    /// Decision 0177's guard for stage 1: on the CPU, no token's last byte comes from a mask
    /// extract. The records have end rules, and the CPU's form makes the lasts' output a zero
    /// constant, so the last-byte masks only it read are not made.
    #[test]
    fn the_cpu_makes_no_lasts_mask() {
        use rerail_compiler::block_native::BlockInst;
        let c = crate::compiled().expect("compiles");
        let k = c.native.ruled_lasts().expect("the records' lasts by end rules");
        let cpu = c.native.cpu_form();
        assert!(matches!(cpu.insts[usize::from(cpu.outputs[k])], BlockInst::Const(0)), "the CPU makes the lasts' mask");
        assert!(cpu.insts.len() < c.native.insts.len(), "the last-byte masks pruned");
        eprintln!("the CPU's form: {} operations, the bit-parallel form's {}", cpu.insts.len(), c.native.insts.len());
    }

    /// Decision 0177's guard for stage 2a: on the CPU, no token's first byte comes from a mask
    /// extract. The loop finds each start, so the starts' output is the zero constant too.
    #[test]
    fn the_cpu_makes_no_starts_mask() {
        use rerail_compiler::block_native::BlockInst;
        let c = crate::compiled().expect("compiles");
        let k = c.native.walked_starts().expect("the records' starts by the loop");
        let cpu = c.native.cpu_form();
        assert!(matches!(cpu.insts[usize::from(cpu.outputs[k])], BlockInst::Const(0)), "the CPU makes the starts' mask");
        let made = c.native.records_made();
        assert!(c.native.extracts.iter().all(|e| made.contains(e)), "every extract the record pass's");
    }

    /// Decision 0177's guard for the token loop's reads: the masks it reads that are made from
    /// the bytes near each byte (`native_cpu_bytes`) it makes where it reads them, as the lean
    /// lexer makes its window's masks and reads a string from its opener; the CPU's body makes
    /// none of them. The skip, the runs, the readers' openers and closers, the tests.
    #[test]
    fn the_cpu_reads_local_tests_at_the_token() {
        use rerail_compiler::block_native::BlockInst;
        let c = crate::compiled().expect("compiles");
        let at = c.native.read_at_tokens();
        let r = c.native.records.as_ref().expect("records");
        assert!(at.contains(&usize::from(r.skip.expect("a skip"))), "the skip from the bytes: {at:?}");
        assert!(at.len() >= 8, "the runs', readers' and tests' masks from the bytes: {at:?}");
        let cpu = c.native.cpu_form();
        for &o in &at {
            assert!(matches!(cpu.insts[usize::from(cpu.outputs[o])], BlockInst::Const(0)), "the CPU makes test {o}'s mask");
        }
        // Tier 2's `\\u` checks read their mask from the bytes too, and a number's chain is
        // cut by the loop (`run … once dot`): the CPU's body makes nothing.
        let made: Vec<String> = cpu.insts.iter().filter(|i| !matches!(i, BlockInst::Const(_))).map(|i| format!("{i:?}")).collect();
        assert!(made.is_empty(), "the CPU's body makes {made:?}");
    }

    /// The token tests a first byte decides (`native_cpu_block::test_filter`, from what their
    /// masks need of the bytes): the `-` not before a digit, the leading `0`. Each a bit of the
    /// token's entry on the CPU, as the lean lexer tests those at those bytes alone.
    #[test]
    fn a_tests_first_byte_filters_it() {
        let c = crate::compiled().expect("compiles");
        let r = c.native.records.as_ref().expect("records");
        let filtered: Vec<Vec<u8>> = r.checks.iter().filter_map(|k| rerail_compiler::native_cpu_block::test_filter(&c.native, k)).map(|n| (0..=255u8).filter(|&b| n.has(b)).collect()).collect();
        assert_eq!(filtered, vec![b"-".to_vec(), b"0".to_vec()]);
    }

    /// The pair rule's table as the lean lexer's nibble tables (`native_cpu_block::pair_nibbles`):
    /// the token loop reads a pair's second byte from a block's word, a bit a byte, not the
    /// table at each token. Each first byte of `.rerail`'s pairs is its own bit.
    #[test]
    fn the_pairs_are_a_blocks_word() {
        let c = crate::compiled().expect("compiles");
        let r = c.native.records.as_ref().expect("records");
        let found: Vec<usize> = r.rules.iter().enumerate().filter_map(|(k, e)| match *e {
            rerail_compiler::records::EndRule::Next(i) => rerail_compiler::native_cpu_block::pair_nibbles(r, &r.nexts[usize::from(i)], k).map(|n| n[0].iter().fold(0u8, |a, &x| a | x).count_ones() as usize),
            _ => None,
        }).collect();
        // `:` `=` `!` `<` `>` `-`: six first bytes, six bits; and `pair empty`'s table of none,
        // no bits (0355: its word is zero, no lookup).
        assert_eq!(found, vec![6, 0]);
    }

    /// A `within(d)` the CPU's windows leave out (`native_cpu_lanes::unguarded`): where another
    /// conjunct is false on the byte it guards when that byte is zero, as the bytes off a file
    /// of the batch are (`zero_kills`). A `.` a byte back needs none; a byte not `a`, or a zero
    /// byte, does.
    #[test]
    fn a_within_goes_where_a_zero_byte_refuses() {
        use rerail_compiler::block_native::BlockInst::{And, EqByte, Not, Prev, Within};
        use rerail_compiler::native_cpu_lanes::{unguarded, zero_kills};
        let c = crate::compiled().expect("compiles");
        let mut n = c.native.clone();
        n.insts = vec![EqByte(b'.'), Prev(0, 1), Within(1), And(1, 2), Not(0), And(4, 2), EqByte(0), And(6, 2), And(3, 0)];
        assert!(zero_kills(&n, 1, 1));
        assert!(!zero_kills(&n, 1, 0));
        assert!(!zero_kills(&n, 4, 0));
        assert!(!zero_kills(&n, 6, 0));
        assert_eq!(unguarded(&n, 3), Some(vec![1]));
        assert_eq!(unguarded(&n, 5), None);
        assert_eq!(unguarded(&n, 7), None);
        // Through the `and`s: the `.` of the outer one's inner one.
        assert_eq!(unguarded(&n, 8), Some(vec![1, 0]));
    }

    /// A `prev`'s bound the CPU's scalar tests leave out (`native_cpu_bytes::zeros_bound`):
    /// where its operand is false on a zero byte, read within `FRONT` bytes before the file,
    /// the zeros there. A `.` needs none; a byte not `.`, or one read further back, does.
    #[test]
    fn a_prevs_bound_goes_where_the_zeros_say_it() {
        use rerail_compiler::block_native::BlockInst::{EqByte, Not};
        use rerail_compiler::native_cpu_bytes::zeros_bound;
        let c = crate::compiled().expect("compiles");
        let mut n = c.native.clone();
        n.insts = vec![EqByte(b'.'), Not(0), EqByte(0)];
        let front = rerail_compiler::native_cpu_block::FRONT as i64;
        assert!(zeros_bound(&n, 0, 1));
        assert!(zeros_bound(&n, 0, front));
        assert!(!zeros_bound(&n, 0, front + 1));
        assert!(!zeros_bound(&n, 1, 1));
        assert!(!zeros_bound(&n, 2, 1));
    }

    /// A class of one run of bytes is a subtract and a compare on the CPU's vectors
    /// (`native_cpu_lanes::set_range`): a digit's `b - '0' <= 9`. Whitespace is no run; a
    /// byte alone is one, a compare.
    #[test]
    fn a_digit_is_a_range() {
        use rerail_compiler::native_cpu_lanes::set_range;
        let set = |bytes: &[u8]| {
            let mut s = [0u64; 4];
            for &b in bytes {
                s[usize::from(b) / 64] |= 1 << (b % 64);
            }
            s
        };
        assert_eq!(set_range(set(b"0123456789")), Some((b'0', b'9')));
        assert_eq!(set_range(set(b"/")), Some((b'/', b'/')));
        assert_eq!(set_range(set(b" \t\n\r")), None);
        assert_eq!(set_range(set(b"0123456789_")), None);
        assert_eq!(set_range(set(b"")), None);
        assert_eq!(set_range(set(&[0xfe, 0xff])), Some((0xfe, 0xff)));
    }

    /// A float's `.` counts (`count(dot) at least 1`, `at least 2`): the mask is its bytes
    /// exactly (`native_cpu_bytes::exact`), so a short number's counts are one vector's, as a
    /// lexer counts the `.` it reads; a mask that looks at other bytes is not.
    #[test]
    fn a_floats_dots_are_its_bytes() {
        let c = crate::compiled().expect("compiles");
        let cpu = &c.native;
        let r = c.program.block_programs[0].records.as_ref().expect("records");
        let counts: Vec<_> = r.checks.iter().filter(|t| t.count && !t.always()).collect();
        let exact: Vec<_> = counts.iter().filter_map(|t| rerail_compiler::native_cpu_bytes::exact(cpu, t.o).map(|n| (t, n))).collect();
        assert!(exact.len() >= 2, "{counts:?}");
        for (t, n) in &exact {
            assert_eq!((n.lo, n.hi, n.ranges()), (0, 0, vec![(b'.', b'.')]), "{t:?}");
        }
        assert!(exact.len() < counts.len(), "a string's counts look past their bytes");
    }

    /// A comment's JSON mark (`json_bad … after comment when count(line_open | block_open) at
    /// least 1`): every comment the reader reads starts with an opener, so the lowering proves
    /// the count holds (`Lowering::implies`) and makes none (`TokenCheck::always`), as a lexer
    /// knows a comment has its opener. It holds where the reader reads a comment, not at a `/`
    /// its other rule takes (`records::test_holds`).
    #[test]
    fn a_comments_opener_is_its_count() {
        let c = crate::compiled().expect("compiles");
        let block = &c.program.block_programs[0];
        let r = block.records.as_ref().expect("records");
        let always: Vec<_> = r.checks.iter().filter(|t| t.always()).collect();
        assert_eq!(always.len(), 1, "{:?}", r.checks);
        let t = always[0];
        assert!(matches!(r.rules[usize::from(t.rule)], rerail_compiler::records::EndRule::Read(_)), "{t:?}");
        // No other count folds: a string's bad bytes are not its opener.
        assert!(r.checks.iter().filter(|x| x.count && !x.always()).count() >= 1);
        let f = b"a(x / 2) // c\n/* d */ b(y / z).";
        let run = block.run(f);
        let holds = |s: usize| rerail_compiler::records::test_holds(r, t, s, s, &run.outputs);
        for (s, want) in [(4, false), (9, true), (14, true), (26, false)] {
            assert_eq!(holds(s), want, "at byte {s} ({:?})", f[s] as char);
        }
    }

    /// The JIT's dispatches are compare trees, as rustc lowers a `match`
    /// (`native_cpu_codegen::switch_trees`), not Cranelift's `br_table`, whose `csdb` barrier
    /// held each rare token (a number, a string, a comment) until its entry's load: 75 cycles
    /// a number against the AOT's 17. The IR keeps its tables (the AOT's `match`).
    #[test]
    fn the_jits_dispatches_are_trees() {
        let c = crate::compiled().expect("compiles");
        assert!(c.parser.tier1.clif.contains(" br_table "), "the IR keeps the rules' table");
        for (name, code) in [("tier 1", &c.parser.tier1.disassembly), ("tier 2", &c.parser.tier2.disassembly)] {
            assert!(!code.contains("csdb"), "{name}: a table's barrier");
        }
    }

    /// The records' layout the compiler chose (`records::RecordLayout`): rr's token loop writes
    /// each token's start, last and code, its only extracts, so on the CPU they are rows, the
    /// lean lexer's `Tok`; tier 2 reads them a row a token and its tapes are the columns'.
    #[test]
    fn rrs_records_are_rows_on_the_cpu() {
        let c = crate::compiled().expect("compiles");
        let t = c.parser.tier1.records.as_ref().expect("the record pass");
        assert_eq!(t.layout, rerail_compiler::records::RecordLayout::Rows);
        // Every first byte of the fused pair rule (punctuation, most of rr's tokens) takes its
        // code from the table, as the lean lexer's `TOKEN`: no word looked up. So does a byte
        // whose reader's other rule reads the same pair table (`/` alone, the comment reader's
        // other), as the lean lexer's `/` arm writes its punctuation (0177). But `<`, whose two
        // pairs (`<=`, `<:`) are two words: its two-byte tokens look theirs up.
        use rerail_compiler::records::EndRule;
        let r = c.native.records.as_ref().expect("records");
        let (kn, _) = t.fused.expect("the fused rules");
        let EndRule::Next(table) = r.rules[kn] else { panic!("the fused pair rule reads a pair table") };
        let pairs = |e: u8| {
            usize::from(e) == kn
                || matches!(r.rules.get(usize::from(e)), Some(&EndRule::Read(i)) if r.readers.get(usize::from(i)).is_some_and(|rd| matches!(r.rules.get(usize::from(rd.other)), Some(&EndRule::Next(o)) if o == table)))
        };
        let pair_bytes: Vec<u8> = (0..=255u8).filter(|&b| r.ends.get(usize::from(b)).is_some_and(|&e| pairs(e)) && b != b'<').collect();
        assert!(pair_bytes.contains(&b'/'), "the comment reader's other reads the pair table");
        assert_eq!(c.parser.tier1.punct_bytes(), pair_bytes);
        // Their codes fit their entries: no load beside the entry's. The entries' codes stay the
        // fused rule's alone.
        assert!(c.parser.tier1.punct_in_entry());
        // Its starts are the lean lexer's (`lean_starts`): a word's or a pair's token clears its
        // own start alone. Its pairs chain (`=>=`), read left to right. `#` (`#include`), whose
        // run starts after it, takes the general tail.
        let (kn, kr) = t.fused.expect("the fused rules");
        assert_eq!(rerail_compiler::native_cpu_block::lean_starts(r, &c.native, r.skip.expect("starts by the loop"), kn, kr), Some(true));
        assert_eq!(t.lean, Some(true));
        let src = ".decl a(x: number)\na(1). // c\na(\"s\").\n";
        let lean = crate::rr::lean::tape(src);
        assert_eq!(c.parser.image(src.as_bytes()).ok().as_deref(), Some(&lean[..]), "rows: the lean tape");
    }

    /// What a rule's count tests need of the bytes, together (`native_cpu_bytes::needs`): a
    /// string's, a control byte or a backslash from the byte before it (`escaped`'s run) to
    /// its last. A string with none of them skips its tests' call, by one vector compare.
    #[test]
    fn a_strings_tests_need_a_control_byte_or_a_backslash() {
        let c = crate::compiled().expect("compiles");
        let r = c.native.records.as_ref().expect("records");
        let mut by_rule: std::collections::BTreeMap<u8, Option<rerail_compiler::native_cpu_bytes::Needs>> = Default::default();
        for k in r.checks.iter().filter(|k| k.count) {
            let n = rerail_compiler::native_cpu_bytes::needs(&c.native, k.o);
            let e = by_rule.entry(k.rule).or_insert(n);
            *e = e.zip(n).map(|(a, b)| a.union(b));
        }
        let shown: Vec<_> = by_rule.values().map(|n| n.map(|n| (n.ranges(), n.lo, n.hi))).collect();
        assert!(shown.contains(&Some((vec![(0, 0x1f), (b'\\', b'\\')], -1, 0))), "{shown:?}");
    }

    /// The code bits of the gap marks tier 2 on the CPU tests itself
    /// (`native_cpu_block::lazy_marks`): the CPU's tier 1 sets none of them.
    fn lazy_bits(block: &rerail_compiler::block_native::BlockNative) -> u32 {
        assert!(block.records.is_some(), "records");
        rerail_compiler::native_cpu_block::lazy_code_bits(block)
    }

    /// Of those, the code bits of the marks with token tests too (`native_cpu_block::lazy_coded`):
    /// the CPU's tier 1 sets their tests' hits, tier 2 tests their gaps.
    fn lazy_coded_bits(block: &rerail_compiler::block_native::BlockNative) -> u32 {
        let r = block.records.as_ref().expect("records");
        let coded = rerail_compiler::native_cpu_block::lazy_coded(r);
        lazy_bits(block) & coded.checked_shl(rerail_compiler::records::MARK_SHIFT).unwrap_or(0)
    }

    /// The interpreter's codes without those bits.
    fn unlazy(codes: &[u32], lazy: u32) -> Vec<u32> {
        codes.iter().map(|c| c & !lazy).collect()
    }

    /// A compiled tier 1's codes against another run's: the same; or, for a file it refuses (a
    /// total set), the same up to the refusal and none or 0s past it, as a refusal leaves the
    /// compiled loop (decision 0177; `ladder::tier1_agrees`'s rule) where the interpreter reads on.
    fn codes_agree(got: &[u32], want: &[u32], refused: bool) -> bool {
        if !refused {
            return got == want;
        }
        let left = got.iter().zip(want).position(|(g, w)| g != w).unwrap_or(got.len());
        got.len() <= want.len() && got[left..].iter().all(|&c| c == 0)
    }

    /// A file refused: any of its totals set (an error output's count, or the loop's flag).
    fn refusal(totals: &[u64]) -> bool {
        totals.iter().any(|&t| t != 0)
    }

    /// Each of `short`'s position lists a prefix of `long`'s: a refused file's records, cut at
    /// the token that refused. Metal's check against the CPU's reads it.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn prefix_of(short: &[Vec<u32>], long: &[Vec<u32>]) -> bool {
        short.len() == long.len() && short.iter().zip(long).all(|(s, l)| l.starts_with(s))
    }

    /// A string's, a comment's and a backtick's tokens end plainly (`plain_end`): no start is
    /// put back after them, as the lean lexer's arms for them put none back. A number's can end
    /// before a word's byte (`12abc`), and `#include`'s inside a run: theirs are put back, the
    /// run's byte alone (`word_end`).
    #[test]
    fn a_strings_end_puts_no_start_back() {
        let c = crate::compiled().expect("compiles");
        let t = c.parser.tier1.records.as_ref().expect("the record pass");
        for b in *b"\"`/" {
            assert_eq!(t.end_at(b), rerail_compiler::native_cpu_block::TokenEnd::Plain, "{:?} ends plainly", b as char);
        }
        for b in *b"07#a(" {
            assert_ne!(t.end_at(b), rerail_compiler::native_cpu_block::TokenEnd::Plain, "{:?} does not", b as char);
        }
        // A number's and `#include`'s last byte is no pair's first (`12.5`'s dot is no run's
        // last): the byte after is put back where it is a run byte alone, as the lean lexer's
        // number arm reads the word after it. A punctuation's can be a pair's first.
        for b in *b"07#" {
            assert_eq!(t.end_at(b), rerail_compiler::native_cpu_block::TokenEnd::Word, "{:?} ends with no pair's first byte", b as char);
        }
        for b in *b"\"/(" {
            assert_ne!(t.end_at(b), rerail_compiler::native_cpu_block::TokenEnd::Word, "{:?} is not of the class", b as char);
        }
    }

    /// The CPU's tier 1 (the JIT) makes the interpreter's records: each file's starts, lasts,
    /// codes and totals, over the samples, a corpus sample and files cut at the blocks' edges.
    #[test]
    fn the_cpus_records_are_the_interpreters() {
        let c = crate::compiled().expect("compiles");
        let block = &c.program.block_programs[0];
        let r = block.records.as_ref().expect("records");
        let lazy = lazy_bits(&c.native);
        assert!(lazy != 0, "no gap mark tier 2 tests itself (`line`, `newline`)");
        let files = corpus();
        let mut inputs: Vec<Vec<u8>> = crate::SAMPLES.iter().map(|s| s.as_bytes().to_vec()).collect();
        inputs.extend(files.iter().step_by(5).map(|f| f.as_bytes().to_vec()));
        for n in [1usize, 63, 64, 65, 127, 128, 129, 200] {
            inputs.push(format!("r(x) :- a(x), {}.\n", "b".repeat(n)).into_bytes());
            inputs.push(format!("{}\"open", " ".repeat(n)).into_bytes());
        }
        // A word on past its block by its next block's first 16 bytes (`short_word`) or more,
        // and one to the file's end.
        for pad in [48usize, 50, 62, 63] {
            for n in [1usize, 2, 15, 16, 17, 18, 31, 80] {
                inputs.push(format!("{}{} x\n", " ".repeat(pad), "w".repeat(n)).into_bytes());
                inputs.push(format!("{}{}", " ".repeat(pad), "w".repeat(n)).into_bytes());
            }
        }
        // Two-byte spellings (`pairs`): runs of them, and one at the file's end. The CPU reads
        // the next byte (`EndRule::Next`); the interpreter's rule is the same, its mask Metal's.
        for t in [":->", "a :-- b.", "x<:y", "a<=:b", "!=>=", "->->", "a <", "b :", "-", "=>=>="] {
            inputs.push(t.as_bytes().to_vec());
            inputs.push(format!("{}{t}", " ".repeat(62)).into_bytes());
        }
        // The lean lexer's starts (`lean_starts`): no start in a run after its first byte, nor
        // at a pair's second. Another rule's token ending where they left one out (a word right
        // after a number or a string, pairs right after a string, in a run of them), and runs of
        // pairs across the blocks' edges.
        for t in ["#include \"x\"\na(1).", "#includex#y", "a(12abc).", "a(\"x\"b).", "a(\"=\"=>=b).", "a(\"x\":-=>=).", "a(1.5x, 2y).", "a(\"<\"<:<=:b)."] {
            for pad in [0usize, 57, 58, 59, 60, 61, 62, 63] {
                inputs.push(format!("{}{t}", " ".repeat(pad)).into_bytes());
            }
        }
        for pad in [59usize, 60, 61, 62, 63] {
            inputs.push(format!("{}=>=>=>=>", " ".repeat(pad)).into_bytes());
        }
        // A word past eight bytes (`violation`) beside words that differ from it past its eighth
        // byte, or are it with a byte more or less (`RecordTables::rest`): at a block's start,
        // across its edge, and as the file's last bytes.
        for t in ["violation", "violatiox", "violationx", "violatio", "violations", "violation_x", "aviolation"] {
            for pad in [0usize, 55, 56, 58, 63] {
                inputs.push(format!("{}{t} .decl violation(x)", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}x. {t}", " ".repeat(pad)).into_bytes());
            }
        }
        // A comment's closer about 64 bytes after its opener, where a short reader's window
        // (`RuleMode::Short`) ends, at each offset in its block; one the file ends in.
        for n in [0usize, 59, 60, 61, 62, 63, 64, 65] {
            for pad in [0usize, 1, 31, 62, 63] {
                let body = "c".repeat(n);
                inputs.push(format!("{}a(x). //{body}\nb(y).\n", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}a(x). /*{body}*/ b(y).\n", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}a(x). //{body}", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}a(x). /*{body}", " ".repeat(pad)).into_bytes());
            }
        }
        // A byte the string's counts need (a tab, a `\u`) at each edge of the gate's vectors
        // (`needs_spans`): 16, 32 and 48 bytes on; and in a string past 64 bytes, at each
        // edge of its 64-byte chunks (`needs_chunks`), in a whole chunk and in the rest.
        for at in [14usize, 15, 16, 17, 30, 31, 32, 33, 46, 47, 48, 62, 63, 64, 65, 79, 126, 127, 128, 129, 150] {
            for odd in ["\t", "\\u12"] {
                inputs.push(format!("a(\"{}{odd}x\", 1).\n", "s".repeat(at)).into_bytes());
                inputs.push(format!("a(\"{}{odd}\", 1).\n", "s".repeat(at)).into_bytes());
                inputs.push(format!("a(\"{}{odd}{}\", 1).\n", "s".repeat(at), "x".repeat(80)).into_bytes());
            }
        }
        // A byte the counts need right after the closer, in its window (`read_fn` makes the
        // tests' masks with the closer's: the window's bits past the closer are not the
        // token's), and the next string's `\u` in the same window.
        for at in [0usize, 1, 30, 60, 61, 62, 63, 64, 65, 127] {
            inputs.push(format!("a(\"{}\"\t, 1).\n", "s".repeat(at)).into_bytes());
            inputs.push(format!("a(\"{}\", \"\\u12\").\n", "s".repeat(at)).into_bytes());
        }
        // A `/` alone, the comment reader's other rule: its code is its kind, no word looked up
        // (no word starts with `/`), and a comment's tail is its own (`known_tail`).
        for pad in [0usize, 1, 62, 63] {
            for t in ["a(x) :- x = 4 / 2.", "x/y", "/", "a / // c\nb", "/ /* c */ /"] {
                inputs.push(format!("{}{t}", " ".repeat(pad)).into_bytes());
            }
        }
        // A token alone in its block, then spaces to and past the block's end (a spent block:
        // the refill reads the next one), after a word, a string, a comment and pairs.
        for gap in [1usize, 30, 61, 62, 63, 64, 65, 126, 127, 128] {
            for t in ["x", "\"s\"", "/* c */", "=>", "\"s\"=>=>", "1.5"] {
                inputs.push(format!("{t}{}y=>=.\n", " ".repeat(gap)).into_bytes());
                inputs.push(format!("{}{t}{}y.\n", " ".repeat(gap), " ".repeat(gap)).into_bytes());
            }
        }
        // A long string with none: no test's function.
        for n in [64usize, 65, 127, 128, 129, 200] {
            inputs.push(format!("a(\"{}\", 1).\n", "s".repeat(n)).into_bytes());
        }
        // A `\` run right before a string's quote: the opener's escape carry, which a short
        // reader leaves to its whole function (`Bytewise::bail`).
        for run in ["\\", "\\\\", "\\\\\\"] {
            for pad in [0usize, 1, 62, 63] {
                inputs.push(format!("{}a({run}\"s\", 1).\n", " ".repeat(pad)).into_bytes());
            }
        }
        // Strings and comments read from the bytes (`native_cpu_bytes`): runs of backslashes
        // across the windows' edges, the run before a window found back to its start; a
        // string or comment over many blocks; the escape at the opener's byte.
        for n in [1usize, 2, 3, 63, 64, 65, 66, 127, 128, 129, 130] {
            for pad in [0usize, 1, 30, 62, 63] {
                let run = "\\".repeat(n);
                inputs.push(format!("{}r(\"{run}\") :- a(x).\n", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}r(\"a{run}\\\"b\") :- a(x).\n", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}r(\"{}\") :- a(x). // {run}\n/* {run} */ b(y).\n", " ".repeat(pad), "x".repeat(n)).into_bytes());
            }
        }
        assert!(!r.rules.iter().any(|x| matches!(x, rerail_compiler::records::EndRule::Pair(_))), "a pair rule reads a mask: the pairs are read from the bytes");
        assert!(r.rules.iter().any(|x| matches!(x, rerail_compiler::records::EndRule::Next(_))), "no pair read from the bytes");
        let refs: Vec<&[u8]> = inputs.iter().map(|f| f.as_slice()).collect();
        let got = c.parser.tier1.run_files(&refs);
        for (k, (f, g)) in refs.iter().zip(&got).enumerate() {
            let want = block.run(f);
            // A refused file (any total set) goes to the fallback: both refuse it, and its
            // records are a prefix, up to the token that refused (decision 0177, "A refusal
            // leaves the loop"): the CPU's stop there, the interpreter's go on.
            let refused = refusal(&want.totals);
            assert_eq!(refusal(&g.totals), refused, "file {k}: the refusal");
            let upto = |x: usize, y: usize| if refused { x.min(y) } else { x.max(y) };
            for (what, e) in [("starts", usize::from(r.starts)), ("lasts", usize::from(r.lasts))] {
                let (x, y) = (&g.positions[e], &want.positions[e]);
                if let Some(i) = (0..upto(x.len(), y.len())).find(|&i| x.get(i) != y.get(i)) {
                    let at = y.get(i).or(x.get(i)).copied().unwrap_or(0) as usize;
                    let text = String::from_utf8_lossy(&f[at.saturating_sub(8).min(f.len())..(at + 12).min(f.len())]).into_owned();
                    let around = |v: &Vec<u32>, w: &Vec<u32>| (i.saturating_sub(2)..(i + 3).min(v.len())).map(|j| (v[j], w.get(j).copied())).collect::<Vec<_>>();
                    panic!("file {k} ({} bytes): {what} {i} the CPU's {:?}, the interpreter's {:?} ({} and {} of them), near {text:?}; the CPU's (start, last) {:?}, the interpreter's {:?}", f.len(), x.get(i), y.get(i), x.len(), y.len(), around(&g.positions[usize::from(r.starts)], &g.positions[usize::from(r.lasts)]), around(&want.positions[usize::from(r.starts)], &want.positions[usize::from(r.lasts)]));
                }
            }
            // The marks with token tests too keep their tests' hits on the CPU
            // (`lazy_coded`): `the_marks_tests_are_the_masks` compares those.
            let (w, gc) = (unlazy(&want.codes, lazy), unlazy(&g.codes, lazy));
            if let Some(i) = (0..upto(gc.len(), w.len())).find(|&i| gc.get(i) != w.get(i)) {
                let at = g.positions[usize::from(r.starts)].get(i).copied().unwrap_or(0) as usize;
                let text = String::from_utf8_lossy(&f[at.saturating_sub(40).min(f.len())..(at + 12).min(f.len())]).into_owned();
                panic!("file {k} ({} bytes): the codes (the gap marks tier 2 tests itself apart) differ at token {i}, byte {at}, the CPU's {:#x}, the interpreter's {:#x}, near {text:?}", f.len(), g.codes[i], w[i]);
            }
            if !refused {
                assert_eq!(g.totals, want.totals, "file {k}: the totals");
            }
        }
    }

    /// Decision 0177's census of the CPU's form: each output, its label, the operations it
    /// reads and those only it reads (what its going would take out).
    #[test]
    #[ignore = "a census: run by hand"]
    fn cpu_outputs_by_operations() {
        let c = crate::compiled().expect("compiles");
        let cpu = c.native.cpu_form();
        let n = cpu.insts.len();
        if let Some(r) = &c.native.records {
            let bytes = |k: usize| (0..256).filter(|&b| usize::from(r.ends[b]) == k).count();
            eprintln!("{} end rules: {}", r.rules.len(), r.rules.iter().enumerate().map(|(k, x)| format!("{x:?} ({} bytes)", bytes(k))).collect::<Vec<_>>().join(", "));
            // Each rule's share of the corpus's tokens (by their first bytes): what a dispatch
            // is laid out for.
            let files = corpus();
            let mut per = vec![0u64; r.rules.len() + 1];
            for f in &files {
                let run = c.program.block_programs[0].run(f.as_bytes());
                for &st in &run.positions[usize::from(r.starts)] {
                    let rule = usize::from(r.ends[usize::from(f.as_bytes()[st as usize])]);
                    per[rule.min(r.rules.len())] += 1;
                }
            }
            let all: u64 = per.iter().sum();
            eprintln!("tokens by rule: {}", per.iter().enumerate().map(|(k, &n)| format!("{k}: {:.1}%", 100.0 * n as f64 / all as f64)).collect::<Vec<_>>().join(", "));
        }
        let reach = |o: usize| {
            let mut seen = vec![false; n];
            let mut todo = vec![usize::from(cpu.outputs[o])];
            while let Some(x) = todo.pop() {
                if x < n && !seen[x] {
                    seen[x] = true;
                    todo.extend(cpu.insts[x].operands());
                }
            }
            seen
        };
        let all: Vec<Vec<bool>> = (0..cpu.outputs.len()).map(reach).collect();
        let mut rows = Vec::new();
        for o in 0..cpu.outputs.len() {
            let mine = all[o].iter().filter(|&&b| b).count();
            let only = (0..n).filter(|&x| all[o][x] && (0..cpu.outputs.len()).all(|p| p == o || !all[p][x])).count();
            let mut label: Vec<String> = c.labels.iter().filter(|(_, k)| usize::from(*k) == o).map(|(l, _)| l.clone()).collect();
            label.dedup();
            if let Some(r) = &c.native.records {
                use rerail_compiler::records::EndRule;
                for rule in &r.rules {
                    match *rule {
                        EndRule::Run(k) if usize::from(k) == o => label.push("run".into()),
                        EndRule::Pair(k) if usize::from(k) == o => label.push("pair".into()),
                        EndRule::Last(k) if usize::from(k) == o => label.push("last".into()),
                        _ => {}
                    }
                }
                for t in r.checks.iter().filter(|t| usize::from(t.o) == o) {
                    let l = format!("test {:?}", t.then);
                    if !label.contains(&l) {
                        label.push(l);
                    }
                }
                if r.skip.map(usize::from) == Some(o) {
                    label.push("skip".into());
                }
                for (m, &(k, w)) in r.marks.iter().enumerate() {
                    if usize::from(k) == o {
                        label.push(format!("recmark{m}(w{w})"));
                    }
                }
            }
            for (i, &m) in c.native.masks.iter().enumerate() {
                if m == o {
                    let uses: Vec<&str> = c.words.lines().filter(|l| l.ends_with(&format!(" {i}")) && (l.contains(" ROW") || l.contains(" CHECK") || l.contains(" MARK"))).map(|l| if l.contains(" ROW") { "ROW" } else if l.contains(" CHECK") { "CHECK" } else { "MARK" }).collect();
                    label.push(format!("mask{i}{:?}", uses.first()));
                }
            }
            let d = c.native.output_delays.get(o).copied().unwrap_or(0);
            if d > 0 {
                label.push(format!("delay{d}"));
            }
            rows.push((only, mine, o, label.join(",")));
        }
        rows.sort_by(|a, b| b.cmp(a));
        eprintln!("the CPU's form: {n} operations, {} outputs", cpu.outputs.len());
        for (x, inst) in cpu.insts.iter().enumerate() {
            eprintln!("  op {x:3} {inst:?}");
        }
        for (only, mine, o, label) in rows {
            eprintln!("  output {o:3} {label:30} reads {mine:4}, only it {only:4}");
        }
    }

    /// Decision 0177: an error by tokens holds where the token loop refuses, and nowhere else: a
    /// byte no token begins with (`a byte no token takes`), a reader's region open at the file's
    /// end, four backticks where a literal's reader finds no opener. Its mask, which Metal makes,
    /// and the loop, which the CPU runs, refuse the same files.
    #[test]
    fn the_loops_errors_are_the_masks() {
        let c = crate::compiled().expect("compiles");
        let block = &c.program.block_programs[0];
        let r = block.records.as_ref().expect("records");
        assert!(!r.untaken.is_empty(), "an error by tokens");
        let files = corpus();
        let mut inputs: Vec<Vec<u8>> = files.iter().map(|f| f.as_bytes().to_vec()).collect();
        for bad in ["\\", "\u{e9}", "'", "$", "?", "\x01", "\\\"x\"", "\u{1F600}"] {
            for pad in [0usize, 61, 63, 64] {
                inputs.push(format!("{}r(x) :- a(x) {bad} b(x).\n", " ".repeat(pad)).into_bytes());
                inputs.push(format!("{}r(x) :- a(x), \"{bad}\" = x.\n", " ".repeat(pad)).into_bytes());
            }
        }
        // Open at the end, and the backticks' runs, across a block's edge.
        for tail in ["\"abc", "/* abc", "`abc", "```abc", "``abc", "``", "````a````", "`a`````", "// abc", "\"a\\\"", "/*/", "`a``b`"] {
            for pad in [0usize, 50, 58, 61, 62, 63, 64, 70] {
                inputs.push(format!("r(x) :- a(x).\n{}{tail}", " ".repeat(pad)).into_bytes());
                inputs.push(format!("r(x) :- a(x).\n{}{tail}\nr(y) :- a(y).\n", " ".repeat(pad)).into_bytes());
            }
        }
        let skip = r.skip.expect("the loop's skip");
        let (mut refused, mut both) = (0, 0);
        for f in &inputs {
            let run = block.run(f);
            let fires = |o: usize| run.outputs[o].iter().any(|&w| w != 0);
            let others = block.total_outputs().into_iter().any(|o| !r.untaken.iter().any(|&u| usize::from(u) == o) && fires(o));
            let mask = others || r.untaken.iter().any(|&o| fires(usize::from(o)));
            let (_, _, untaken, read, _) = rerail_compiler::records::walk(r, skip, f, &run.outputs);
            let loop_ = others || untaken || read;
            assert_eq!(mask, loop_, "{:?}: refused by the masks {mask}, with the loop's error {loop_}", String::from_utf8_lossy(&f[..f.len().min(80)]));
            refused += usize::from(mask);
            both += 1;
        }
        eprintln!("{both} files, {refused} refused by both");
    }

    /// Decision 0177's guard for the marks: none is a start mask. The marks between tokens are
    /// the records' gap marks, lifted by the record pass; the marks of one word and the token
    /// marks are the word alone or their terms at the token; tier 2 reads no mask of any mark.
    #[test]
    fn the_marks_are_not_start_masks() {
        let c = crate::compiled().expect("compiles");
        let r = c.native.records.as_ref().expect("records");
        assert!(r.gaps != 0, "gap marks");
        assert!(r.extra.is_some(), "the extra's mark");
        // `TOKEN m=1+k` would read mask `k` for any mark; `m=128` is the extras' alone.
        let any_mark = |l: &&str| l.contains(" TOKEN m=") && !l.contains(" TOKEN m=128 ");
        assert!(!c.words.lines().any(|l| any_mark(&l)), "tier 2 reads no mask of any mark");
        assert!(!r.unless.is_empty(), "a line break after a `,` is the gap pass's, not a mask's");
        assert!(r.unplaced != 0, "a float the file ends with is the gap pass's, not a mask's");
    }

    /// The marks' token tests (decision 0177): `float_after` and `json_bad` from checks at their
    /// tokens, as a lexer marks what it reads, are the masks they replace. The old form (the
    /// masks, from the regions; a `-` a pair's second byte, as `:-`, none: the tests take a `-`
    /// token) and the new give the same tape or refusal through tier 2; the CPU's and Metal's
    /// codes are the interpreter's, over the corpus and each case's edges: a JSON string's bad
    /// bytes, a comment, a `-`, a leading zero, a form feed, a float before a comment and at the
    /// file's end.
    #[test]
    fn the_marks_tests_are_the_masks() {
        let new = crate::compiled().expect("compiles");
        let old_text = crate::GRAMMAR
            .replace("mark float_after = empty, after number when count(dot) at least 1,", "mask frac_last  = frac & ~next(frac)\nmark float_after = frac_last,")
            .lines()
            .filter(|l| !(l.starts_with("                   after ") || l.starts_with("                   at ")))
            .map(|l| if l.starts_with("mark json_bad ") { "mask in_str      = f_str & ~o_str\nmark json_bad    = (in_str & ctl) | (in_str & escaped & ~jesc) | (in_str & uesc & ~h4) | o_line | o_block | (code & ff) | (code & minus & ~next(digit) & ~second) | (int & zero & ~prev(int) & next(int))\n".to_string() } else { format!("{l}\n") })
            .collect::<String>();
        assert!(!old_text.contains(" after string when"), "the old form has the masks alone");
        let old = crate::compile_text(&old_text, rerail_compiler::bitlex_masks::Shape::Rules).expect("the old form compiles");
        // `json_bad`'s tests alone, no gap (`ff`): the bits the CPU's tier 1 sets of it, its
        // gap tier 2's (`lazy_coded`).
        let checks_text = crate::GRAMMAR.replace("mark json_bad    = ff,\n", "mark json_bad    = empty,\n");
        assert!(checks_text != crate::GRAMMAR, "json_bad's gap is a form feed");
        let checks = crate::compile_text(&checks_text, rerail_compiler::bitlex_masks::Shape::Rules).expect("the tests alone compile");
        let r = new.program.block_programs[0].records.as_ref().expect("records");
        assert!(r.checks.iter().any(|c| matches!(c.then, rerail_compiler::records::CheckThen::After(_))), "a mark after its token");
        assert!(r.checks.iter().any(|c| matches!(c.then, rerail_compiler::records::CheckThen::At(_))), "a mark at its token");
        assert!(r.tested_only != 0, "float_after reads no mask");
        // No region on the CPU: its form calls no walk (the readers read them, decision 0177).
        assert!(new.native.cpu_form().walks.is_empty(), "the CPU walks the regions");
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        let cases = [
            "r(x) :- x = ps - fe.\n",
            "r(x) :- x = ps - fe, y = \"\\\\s*\".\n",
            "r(x) :- x = {\"a\": \"b\tc\"}.\n",
            "r(x) :- x = {\"a\": \"b\\qc\"}.\n",
            "r(x) :- x = {\"a\": \"\\u12\"}.\n",
            "r(x) :- x = {\"a\": \"\\u12ab\", \"b\": 1}.\n",
            "r(x) :- x = {\"a\": // c\n 1}.\n",
            "r(x) :- x = {\"a\": /* c */ 1}.\n",
            "r(x) :- x = {\"a\": -1, \"b\": - 1}.\n",
            "r(x) :- x = {\"a\": 01, \"b\": 0.5, \"c\": 0}.\n",
            "r(x) :- x = {\"a\":\x0c1}.\n",
            "r(x) :- x = 1.5 /* c */ , y = 2.\n",
            "r(x) :- x = 1.5",
            "r(x) :- x = 1.5 // c",
            "r(x) :- x = -0.25.\n",
            "r(x) :- x = \"a-b\", y = \"0\\\\\".\n",
            "r(x) :- x = 1. // \\u12\n",
            "r(x) :- x = {\"a\": 1 /* \\u12 */}.\n",
            "r(x) :- x = \"\\u12\".\n",
        ];
        for pad in [0usize, 40, 55, 60, 63] {
            for c in cases {
                inputs.push(format!("{}{c}", " ".repeat(pad)).into_bytes());
            }
        }
        // A form feed in a gap across blocks: the refill folds a passed block's bits, and the
        // bits before the cursor's block (a backtick literal's, a comment's) by the bytes; the
        // last start's block's bits from its carried words, the start at its block's end too.
        for pad in [0usize, 30, 60, 61, 62, 63] {
            let p = " ".repeat(pad);
            inputs.push(format!("{p}r(x) :- x = {{\"a\":\x0c{}1}}.\n", " ".repeat(70)).into_bytes());
            inputs.push(format!("{p}r(x) :- x = `\x0c{}`, y = 1.\n", "a".repeat(80)).into_bytes());
            inputs.push(format!("{p}r(x) :- x = {{\"a\": /* \x0c{} */ 1}}.\n", "c".repeat(80)).into_bytes());
        }
        let files = corpus();
        // Every fifth corpus file, and `syntax-literals.rerail`, which a number of 19 digits refuses
        // (`hex_pow(15, 1152921504606846976)`, the number rule's escape): a refusal mid-file.
        let refused = |f: &String| f.contains("hex_pow(15, 1152921504606846976)");
        assert!(files.iter().any(refused), "syntax-literals.rerail is in the corpus");
        inputs.extend(files.iter().enumerate().filter(|(k, f)| k % 5 == 0 || refused(f)).map(|(_, f)| f.as_bytes().to_vec()));
        inputs.extend(crate::SAMPLES.iter().map(|s| s.as_bytes().to_vec()));
        let refs: Vec<&[u8]> = inputs.iter().map(|f| f.as_slice()).collect();
        let cpu = new.parser.tier1.run_files(&refs);
        let lazy = lazy_bits(&new.native);
        let coded = lazy_coded_bits(&new.native);
        assert!(coded != 0, "json_bad: its gap tier 2's, its tests' hits tier 1's");
        let mut taken = [0usize; 2];
        for (k, f) in refs.iter().enumerate() {
            let (a, b) = (new.program.block_programs[0].run(f), old.program.block_programs[0].run(f));
            // Tier 2's result, the tape or the refusal: the old form checks some of `json_bad`'s
            // alternatives in tier 2 (`CHECK`, a look-ahead), the new marks their tokens.
            let (x, y) = (rerail_compiler::stream::run(&new.program, f, &a), rerail_compiler::stream::run(&old.program, f, &b));
            assert_eq!(x.is_ok(), y.is_ok(), "file {k} {:?}: taken by the tests' marks as by the masks", String::from_utf8_lossy(&f[..f.len().min(80)]));
            if let (Ok(x), Ok(y)) = (&x, &y) {
                assert!(x == y, "file {k}: the same tape");
            }
            if k < 5 * cases.len() {
                taken[usize::from(x.is_ok())] += 1;
            }
            // The CPU's codes: the interpreter's, but the lazy marks' gaps (tier 2's).
            let tested = checks.program.block_programs[0].run(f);
            let want: Vec<u32> = a.codes.iter().zip(&tested.codes).map(|(&c, &t)| c & !lazy | t & coded).collect();
            let refused = a.totals.iter().any(|&t| t != 0);
            assert!(codes_agree(&cpu[k].codes, &want, refused), "file {k}: the CPU's codes ({} of {}), up to a refusal", cpu[k].codes.len(), want.len());
            // The CPU's tier 2 takes the gaps: its tape or refusal is the interpreter's, over the
            // cases (the corpus: its tape where it takes the file; two files of 500 KB it
            // refuses as before, the work queue's).
            let image = new.parser.image(f);
            let cased = k < 5 * cases.len() + 18;
            assert!(!cased || image.is_ok() == x.is_ok(), "file {k} {:?}: the CPU's tier 2 takes it as the interpreter: {:?} / {:?}", String::from_utf8_lossy(&f[..f.len().min(80)]), image.as_ref().err(), x.as_ref().err());
            if let (Ok(i), Ok(x)) = (&image, &x) {
                assert!(*i == rerail_compiler::stream::image(x, &new.program.readings.extras, rerail_tape_format::RR_TAG, f), "file {k}: the CPU's tape");
            }
            assert_eq!(cpu[k].totals, a.totals, "file {k}: the CPU's refusals");
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            use rerail_metal::block_program::serial::{Segments, run_serial_split};
            let batch: Vec<rerail_compiler::block::BatchFile<'_>> = refs.iter().map(|b| rerail_compiler::block::BatchFile { bytes: b, program: 0 }).collect();
            // No device here: no Metal run to hold to the CPU's.
            let on = if no_device() { Vec::new() } else { run_serial_split(&[&new.native], &batch, Segments { blocks: 29, warm: 1 }).expect("Metal runs tier 1").run.files };
            // Metal sets the gap marks tier 2 on the CPU tests itself: its codes are the
            // interpreter's whole, up to a refusal (`codes_agree`), as the CPU's.
            for (k, m) in on.iter().enumerate() {
                let interp = new.program.block_programs[0].run(refs[k]);
                let refused = interp.totals.iter().any(|&t| t != 0);
                assert!(codes_agree(&m.codes, &interp.codes, refused), "file {k}: Metal's codes ({} of {}), up to a refusal", m.codes.len(), interp.codes.len());
                let (mc, cc) = (unlazy(&m.codes, lazy), unlazy(&cpu[k].codes, lazy));
                assert!(codes_agree(&cc, &mc, refused) || codes_agree(&mc, &cc, refused), "file {k}: Metal's codes, the CPU's");
                assert_eq!(m.totals, cpu[k].totals, "file {k}: Metal's refusals");
            }
        }
        assert!(taken[0] > 0 && taken[1] > 0, "the cases both refused and taken: {taken:?}");
        eprintln!("{} files: the same marks; of the cases {} refused, {} taken", refs.len(), taken[0], taken[1]);
    }

    /// A number's check at its token (`escape … at number when …`, decision 0177): an integer
    /// of 19 digits or more refuses the file, a float's digits never, on the interpreter, the CPU and Metal
    /// alike, with the token across a block's edge and longer than a block.
    #[test]
    fn a_numbers_checks_are_at_its_token() {
        let c = crate::compiled().expect("compiles");
        let r = c.native.records.as_ref().expect("records");
        assert_eq!(r.checks.iter().filter(|c| c.then == rerail_compiler::records::CheckThen::Escape).count(), 1, "a run, the number rule's");
        let mut cases: Vec<(String, bool)> = Vec::new();
        for pad in [0usize, 30, 40, 44, 50, 55, 58, 60, 63] {
            let head = format!(".decl a(x: number)\n// {}\n", "x".repeat(pad));
            cases.push((format!("{head}a(123456789012345678).\n"), true));
            cases.push((format!("{head}a(1234567890123456789).\n"), false));
            cases.push((format!("{head}a(0000000000000000001).\n"), false));
            cases.push((format!("{head}a(123456789012345678.5).\n"), true));
            cases.push((format!("{head}a(1234567890123456789.5).\n"), true));
            cases.push((format!("{head}a(1.{}).\n", "7".repeat(80)), true));
            cases.push((format!("{head}a(1.{}.5).\n", "7".repeat(80)), false));
            cases.push((format!("{head}a(1.2.3).\n"), false));
        }
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
        let refs: Vec<&[u8]> = cases.iter().map(|(s, _)| s.as_bytes()).collect();
        for (src, taken) in &cases {
            let interp = rerail_compiler::stream::run(&c.program, src.as_bytes(), &c.program.block_programs[0].run(src.as_bytes()));
            assert_eq!(interp.is_ok(), *taken, "{src:?}: the interpreter");
            assert_eq!(c.parser.image(src.as_bytes()).is_ok(), *taken, "{src:?}: the CPU");
        }
        metal_refuses_as_the_cpu(c, &refs, &cases);
    }

    /// Metal's tier 1 refuses each of `cases` where the CPU's does (`a_numbers_checks_are_at_its_token`).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn metal_refuses_as_the_cpu(c: &crate::Compiled, refs: &[&[u8]], cases: &[(String, bool)]) {
        if no_device() {
            return;
        }
        use rerail_compiler::block;
        use rerail_metal::block_program::serial::{Segments, run_serial_split};
        let want = c.parser.tier1.run_files(refs);
        let batch: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
        let got = run_serial_split(&[&c.native], &batch, Segments { blocks: 29, warm: 1 }).expect("Metal runs tier 1");
        for ((g, w), (src, _)) in got.run.files.iter().zip(&want).zip(cases) {
            assert_eq!(g.totals.iter().any(|&t| t != 0), w.totals.iter().any(|&t| t != 0), "{src:?}: Metal refuses as the CPU");
        }
    }

    /// No device lane off macOS and Linux.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn metal_refuses_as_the_cpu(_: &crate::Compiled, _: &[&[u8]], _: &[(String, bool)]) {}

    /// A line break after a `,` is none (`mark newline = lf unless after ","`): a list goes on
    /// over lines, comments between, as the engine reads it; a break after any other token ends
    /// a setting.
    #[test]
    fn a_line_break_after_a_comma_is_none() {
        let c = crate::compiled().expect("compiles");
        let cases = [
            ".decl a(x: number, y: number)\na(x, y) :- b(x),\n  c(y).\n",
            ".decl a(x: number,\n  y: number)\n",
            ".decl a(x: number, // first\n  y: number)\n",
            ".decl a(x: number, /* a\n b */\n  y: number)\n",
            ".decl a(x: number) // a\n\n.output a\n",
            "\n\n// head\n.decl a(x: number)\n.limit 3,\n",
            ".limit 3\n.decl a(x: number)\n",
        ];
        for src in cases {
            let interp = rerail_compiler::stream::run(&c.program, src.as_bytes(), &c.program.block_programs[0].run(src.as_bytes()));
            let engine = std::panic::catch_unwind(|| crate::engine_tape(src)).ok();
            match (c.parser.image(src.as_bytes()), engine) {
                (Ok(t), Some(e)) => {
                    assert!(interp.is_ok(), "{src:?}: the interpreter takes it");
                    assert_eq!(t, e, "{src:?}: the engine's tape");
                }
                (Ok(_), None) => panic!("{src:?}: taken, the engine refuses it"),
                (Err(_), _) => assert!(interp.is_err(), "{src:?}: refused, the interpreter takes it"),
            }
        }
    }

    /// A float read as the engine reads one: digits, a `.`, digits. A `.` after a fraction is
    /// no float's: `1.2.3` is `1.2`, `.` and `3`, and `1.2.3.4` is `1.2`, `.` and `3.4`. Every
    /// case is the engine's tape, or refused by both.
    #[test]
    fn chained_floats_read_as_the_engine() {
        let c = crate::compiled().expect("compiles");
        let machine = crate::compile_with(rerail_compiler::bitlex_masks::Shape::Machine).expect("compiles as one machine");
        let mut cases = Vec::new();
        for pad in [0usize, 40, 50, 55, 58, 60] {
            let head = format!(".decl a(x: number)\n// {}\n", "x".repeat(pad));
            cases.push((format!("{head}a(1.5).\n"), true));
            cases.push((format!("{head}a(x) :- a(y), x = y + 12.25 * 3.0.\n"), true));
            cases.push((format!("{head}a(x) :- a(y), x = y - 0.125.\n"), true));
            // A chain in a clause is the engine's syntax error too; in a setting, its floats
            // and dots: `1.2`, `.`, `3.4`, `.`, `5`.
            cases.push((format!("{head}a(1.2.3).\n"), false));
            cases.push((format!("{head}a(x) :- a(y), x = 10.25.5.\n"), false));
            cases.push((format!("{head}.scope 1.2.3.4.5\na(1).\n"), true));
            cases.push((format!("{head}.mode 10.25.5 x\na(1).\n"), true));
            cases.push((format!("{head}a(x) :- a(y), x = -1.5, y = -20.\n"), true));
            cases.push((format!("{head}.limit 1.5\na(1).\n"), true));
            cases.push((format!("{head}.options {{\"a\": 1.5, \"b\": -0.25, \"c\": 3}}\n"), true));
            cases.push((format!("{head}a(1.5, 1234.5, 2)."), true));
        }
        // A float the file ends with: no token after it to mark.
        cases.push((".decl a(x: number)\n.limit 1.5".to_string(), false));
        cases.push((".decl a(x: number)\n.limit 1.5 // a comment".to_string(), false));
        for (src, taken) in &cases {
            let interp = rerail_compiler::stream::run(&c.program, src.as_bytes(), &c.program.block_programs[0].run(src.as_bytes()));
            assert_eq!(interp.is_ok(), *taken, "{src}: the interpreter {:?}", interp.as_ref().err());
            for p in [&c.parser, &machine.parser] {
                match p.image(src.as_bytes()) {
                    Ok(t) => {
                        assert!(*taken, "{src}: taken");
                        assert_eq!(t, crate::engine_tape(src), "{src}: the engine's tape");
                    }
                    Err(_) => assert!(!*taken, "{src}: refused"),
                }
            }
        }
    }

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

    /// Cranelift's optimizer on `.rerail`'s tier 1 (doc 28): tier 1 compiled with it and without, in
    /// one process, timed in turns over the corpus, each its best round.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn rr_tier1_optimizer() {
        use std::sync::atomic::Ordering::Relaxed;
        let c = crate::compiled().expect("compiles");
        let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
        // The compiles' options are the run's (decision 0442).
        let _run = rerail_context::test_run();
        let options = rerail_compiler::native_cpu_codegen::options();
        let tier1: Vec<(&str, rerail_compiler::native_cpu_block::CompiledBlock)> = [("off", false), ("on", true)]
            .into_iter()
            .map(|(label, on)| {
                options.optimize_tier1.store(on, Relaxed);
                (label, rerail_compiler::native_cpu_block::compile(&c.native).expect("tier 1"))
            })
            .collect();
        options.optimize_tier1.store(false, Relaxed);
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
        let mut native = rerail_compiler::native_ir::lower(&c.program, 0, 0).expect("lowers");
        native.tag = rerail_tape_format::RR_TAG;
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
            v.sort_by_key(|b| std::cmp::Reverse(b.1));
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
            one.words(b, &mut tape).map(|_| tape.image()).map_err(|r| r.pc)
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
            let p = Parser::from_parts(rerail_compiler::native_cpu_block::compile(&native.block_programs[0]).expect("tier 1"), tier2);
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
                let got = p.words(b, &mut tape).map(|_| tape.image()).map_err(|r| r.pc);
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

    /// Throughput over the repository's `.rerail` files, a pass's mean of `REPEAT`, CPU (this thread)
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
        let tokens: usize = files.iter().map(|f| super::rr::engine_tape(f).len()).sum();
        // (CPU, wall) a pass of `f`, its last pass's tables checked.
        let time = |warm: usize, f: &mut dyn FnMut() -> Option<Vec<Vec<u8>>>| {
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
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
                let mut out: Vec<Option<Vec<u8>>> = vec![None; want.len()];
                let (mut np, mut oc) = (0, 0);
                parts(c, &batch, &digests, 0, &mut |secs, at, n, res| {
                    gpu += secs;
                    np += 1;
                    let t = thread_cpu();
                    for i in 0..n {
                        out[at + i] = Some(match res {
                            Some(res) => rerail_metal::tier2_on::image_on(p, res, i).unwrap_or_else(|_| want[at + i].clone()),
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
            let n: usize = files.iter().map(|f| super::rr::engine_tape(f).len()).sum();
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

    /// CPU time over the repository's `.rerail` files: the compiled grammar against the engine's
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
                } else if p.extension().is_some_and(|x| x == "rerail") {
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
        let engine = run(&|s| super::rr::engine_tape(s).len());
        let mb = bytes as f64 / 1e6;
        let load = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
        eprintln!("{} files, {:.2} MB, {ours} parsed by the grammar; compile {compile:.2?} CPU", files.len(), mb);
        eprintln!("  grammar: {compiled:.3?} CPU a pass, {:.1} MB/s", mb / compiled.as_secs_f64());
        eprintln!("  engine:  {engine:.3?} CPU a pass, {:.1} MB/s", mb / engine.as_secs_f64());
        eprintln!("  load: {load}");
    }

    /// This thread's CPU time (user and sys), to the nanosecond.
    fn thread_cpu() -> std::time::Duration {
        rerail_prof::thread_cpu()
    }

    /// The repository's `.rerail` files.
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
                } else if p.extension().is_some_and(|x| x == "rerail") {
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
        // Its counts are the run's (decision 0442).
        let _run = rerail_context::test_run();
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
                let out = tape.image();
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
            room += tape.room_nodes();
            if p.words(&batch, &mut tape).is_ok() {
                used += tape.counts[0] as usize;
            }
        }
        let retries = rerail_compiler::native_cpu_codegen::options().room_retries.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!("tape nodes: {room} room, {used} written ({:.1}x); {retries} runs again on the bound's tape (all passes)", room as f64 / used.max(1) as f64);
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

#[cfg(test)]
mod json_probe {
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A grammar's program, as `compile_padded` makes it, without the CPU's code (the
    /// interpreter's input), its kinds by their own ids.
    fn program(g: &str) -> Result<rerail_compiler::program::Words, String> {
        let stmts = ::bitlex::syntax::parse(g)?;
        let own: RefCell<HashMap<String, u16>> = RefCell::new(HashMap::new());
        let id = |name: &str| -> u16 {
            let mut own = own.borrow_mut();
            let next = 1 + own.len() as u16;
            *own.entry(name.to_owned()).or_insert(next)
        };
        let kind = |n: &str| Some(if n == "ERROR" { u16::MAX } else { id(n) });
        let field = |n: &str| Some(id(n.trim_start_matches("field ")));
        let symbols = rerail_compiler::bitlex_masks::Symbols { kind: &kind, field: &field };
        let compiled = rerail_compiler::bitlex_masks::compile_shaped(&stmts, &[rerail_compiler::bitlex_masks::FALLBACK], Some(&symbols), rerail_compiler::bitlex_masks::Shape::Rules)?;
        rerail_compiler::rir_asm::assemble(&compiled.text())
    }

    /// A probe (JSON step 1): the token-loop JSON (`json_parser/json.bitlex`) against the ladder's
    /// step 14, both by the interpreter: the same nodes on each input.
    #[test]
    #[ignore = "a probe: run by hand"]
    fn json_loop_against_step_14() {
        let new = program(include_str!("../../json_parser/json.bitlex")).unwrap_or_else(|e| panic!("json.bitlex: {e}"));
        let old = program(include_str!("../../json_parser/json-rfc.bitlex")).unwrap_or_else(|e| panic!("json-rfc: {e}"));
        let r = new.block_programs[0].records.as_ref().expect("records");
        eprintln!("rules {:?}\nreaders {:?}", r.rules, r.readers);
        let inputs: Vec<&[u8]> = vec![
            br#"{"a": [1, -2.5e3, true, null], "b\n": {"c": "d\u0041x"}}"#,
            br#""""#,
            br#""\\""#,
            br#"["a\"b", "\\\"", "x"]"#,
            br#"  [ 1 , 2 ]  "#,
            br#"{"k":"v"}"#,
            br#""open"#,
            br#"[01]"#,
            br#"[1.]"#,
            br#"tru"#,
            br#"["\q"]"#,
        ];
        // JSONTestSuite's files and the corpus's small ones (`RERAIL_JSON_SUITE`,
        // `RERAIL_JSON_CORPUS`).
        let mut owned: Vec<(String, Vec<u8>)> = Vec::new();
        for var in ["RERAIL_JSON_SUITE", "RERAIL_JSON_CORPUS"] {
            let Ok(dir) = std::env::var(var) else { continue };
            let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the directory").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
            paths.sort();
            for p in paths {
                let b = std::fs::read(&p).expect("reads");
                if b.len() < 1 << 20 {
                    owned.push((p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), b));
                }
            }
        }
        let quiet = !owned.is_empty();
        let inputs: Vec<(String, &[u8])> = inputs.into_iter().map(|f| (String::from_utf8_lossy(f).into_owned(), f)).chain(owned.iter().map(|(n, b)| (n.clone(), b.as_slice()))).collect();
        let (mut bad, mut ok, mut refused) = (0, 0, 0);
        for (name, f) in inputs {
            let a = rerail_compiler::stream::run(&new, f, &new.block_programs[0].run(f)).map(|n| n.iter().map(|x| (x.kind, x.start, x.end, x.size)).collect::<Vec<_>>());
            let b = rerail_compiler::stream::run(&old, f, &old.block_programs[0].run(f)).map(|n| n.iter().map(|x| (x.kind, x.start, x.end, x.size)).collect::<Vec<_>>());
            let same = match (&a, &b) {
                (Ok(x), Ok(y)) => x == y,
                (Err(_), Err(_)) => true,
                _ => false,
            };
            if !same {
                bad += 1;
            } else if a.is_ok() {
                ok += 1;
            } else {
                refused += 1;
            }
            if !same || !quiet {
                eprintln!("{} {name:?}: new {:?}, step 14 {:?}", if same { "same" } else { "DIFF" }, a.as_ref().map(|n| n.len()).map_err(|e| e.why), b.as_ref().map(|n| n.len()).map_err(|e| e.why));
            }
        }
        eprintln!("{ok} parsed the same, {refused} refused by both, {bad} differ");
        assert_eq!(bad, 0);
    }

    /// JSON's files: the probe's, and JSONTestSuite's and the corpus's under 1 MB where
    /// `RERAIL_JSON_SUITE` and `RERAIL_JSON_CORPUS` name them.
    fn json_files() -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for var in ["RERAIL_JSON_SUITE", "RERAIL_JSON_CORPUS"] {
            let Ok(dir) = std::env::var(var) else { continue };
            let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the directory").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
            paths.sort();
            out.extend(paths.iter().map(|p| std::fs::read(p).expect("reads")).filter(|b| b.len() < 1 << 20));
        }
        out
    }

    /// JSON's parser with tier 2 a function a grammar rule, as rr's (`by_rules`, optimized).
    fn json_parser(n: &rerail_compiler::native_ir::Structure) -> rerail_compiler::native_cpu_stream::Parser {
        let cut = rerail_compiler::native_cpu_clusters::Cut { near: true, unsigned: true, speed: true, ..rerail_compiler::native_cpu_clusters::by_rules(n) };
        rerail_compiler::native_cpu_stream::Parser::with_cut(n, &cut).expect("JSON's parser")
    }

    /// Lowering A against B (decision 0323): tier 1 and tier 2 over JSON's corpus
    /// (`RERAIL_JSON_CORPUS`), each the least of `RR_ROUNDS` rounds, instructions and cycles.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn json_a_against_b() {
        use rerail_compiler::records::Split;
        let retired = rerail_compiler::native_cpu_codegen::retired;
        let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
        let lower = |at: Split| {
            let mut n = rerail_compiler::native_ir::lower(&json_split(at), 0, 0).expect("lowers");
            for p in &mut n.block_programs {
                p.pad = rerail_compiler::native_cpu_block::BACK as u16;
            }
            n
        };
        let (pa, pb) = (json_parser(&lower(Split::Tier1)), json_parser(&lower(Split::Tier2)));
        let files: Vec<Vec<u8>> = json_files().into_iter().filter(|f| pa.image(f).is_ok()).collect();
        let bytes: usize = files.iter().map(Vec::len).sum();
        let mut rows = Vec::new();
        for (name, p) in [("A (tier 1 splits)", &pa), ("B (tier 2 splits)", &pb)] {
            let mut batches: Vec<_> = files.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_slice()])).collect();
            for b in &mut batches {
                b.call(&p.tier1);
            }
            let mut tapes: Vec<_> = batches.iter().map(|b| p.tape(b)).collect();
            let mut best = [(u64::MAX, u64::MAX); 2];
            for _ in 0..rounds {
                let (i0, c0) = retired();
                for b in &mut batches {
                    b.call(&p.tier1);
                }
                let (i1, c1) = retired();
                for (b, t) in batches.iter().zip(&mut tapes) {
                    let _ = p.words(b, t);
                }
                let (i2, c2) = retired();
                best[0] = (best[0].0.min(i1 - i0), best[0].1.min(c1 - c0));
                best[1] = (best[1].0.min(i2 - i1), best[1].1.min(c2 - c1));
            }
            rows.push((name, best));
        }
        eprintln!("{} files, {bytes} bytes; least of {rounds} rounds", files.len());
        for (name, [t1, t2]) in rows {
            let per = |x: u64| x as f64 / bytes as f64;
            eprintln!("{name:>18}: tier 1 {:.3} instructions {:.3} cycles a byte; tier 2 {:.3} / {:.3}; both {:.3} cycles a byte ({:.2}M cycles)", per(t1.0), per(t1.1), per(t2.0), per(t2.1), per(t1.1 + t2.1), (t1.1 + t2.1) as f64 / 1e6);
        }
    }

    /// The split tier 1 stores a block's word only where a reader reads it from the rows (decision
    /// 0347): tier 2's masks, the marks, and the newlines the line index is made from (decision 0493). Its totals are its refusals' flags, each one running
    /// word (simdjson's `error` register), no row: the token loop makes the rest from the bytes.
    #[test]
    fn json_split_stores_only_what_rows_are_read_for() {
        use rerail_compiler::records::Split;
        let n = rerail_compiler::native_ir::lower(&json_split(Split::Tier2), 0, 0).expect("lowers");
        let p = &n.block_programs[0];
        let r = p.records.as_ref().expect("records");
        let unstored: Vec<usize> = p.records_made().into_iter().chain(p.rowless_totals()).collect();
        let marks: Vec<usize> = r.marks.iter().filter(|m| m.0 != rerail_compiler::records::WORD_ALONE).map(|m| usize::from(m.0)).collect();
        let stored: Vec<usize> = (0..p.outputs.len()).filter(|k| !unstored.contains(k)).collect();
        assert!(!p.totals.is_empty(), "the refusals' totals");
        for k in &stored {
            assert!(p.masks.contains(k) || marks.contains(k) || p.newlines == Some(*k), "output {k} stored a block, read by no row's reader");
        }
        for t in &p.totals {
            assert!(!stored.contains(t) || p.masks.contains(t) || marks.contains(t), "total {t} stored a block");
        }
        eprintln!("{} outputs; stored a block: {stored:?}; totals {:?}", p.outputs.len(), p.totals);
    }

    /// JSON's tier 1 is structured control (0363): its passes' graphs are reducible, every loop
    /// entered at its head alone, so its Rust form (`clif_rust`) is nested loops, as the lean
    /// lexer's, with no loop over a label switch (a `goto`'s translation, which rustc compiles to
    /// a jump table every transition passes). The Rust form is NEON's (`clif_rust`'s target), so
    /// the test is aarch64's: x86-64's tier 1 has its own vector forms (a movemask, `vhigh_bits`).
    #[test]
    #[cfg(target_arch = "aarch64")]
    fn json_tier1_has_no_label_switch() {
        use rerail_compiler::records::Split;
        let mut n = rerail_compiler::native_ir::lower(&json_split(Split::Tier2), 0, 0).expect("lowers");
        for p in &mut n.block_programs {
            p.pad = rerail_compiler::native_cpu_block::BACK as u16;
        }
        // The compile's options are the run's (decision 0442).
        let _run = rerail_context::test_run();
        rerail_compiler::native_cpu_codegen::options().keep_ir.store(true, std::sync::atomic::Ordering::Relaxed);
        let kept = json_parser(&n);
        let src = rerail_compiler::clif_rust::rust_source(&kept).expect("to Rust");
        let switches = src.matches("a dispatch's label is one of its entries").count();
        assert_eq!(switches, 0, "tier 1 has {switches} loops over a label switch: a loop entered past its head");
    }

    /// JSON's tier 1 by pass (decision 0323's next measure): over JSON's corpus
    /// (`RERAIL_JSON_CORPUS`) under B, each pass's instructions and cycles a byte, the least of
    /// `RR_ROUNDS`, against simdjson's stage 1 on the same files. `RERAIL_JSON_RUST=<path>`:
    /// tier 1 written as Rust there (`clif_rust`, the ahead-of-time form), to read its shape;
    /// `RERAIL_JSON_ASM=<path>`, its machine code.
    #[test]
    #[ignore = "a benchmark: run by hand"]
    fn json_tier1_by_pass() {
        use rerail_compiler::records::Split;
        let retired = rerail_compiler::native_cpu_codegen::retired;
        let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(9);
        let mut n = rerail_compiler::native_ir::lower(&json_split(Split::Tier2), 0, 0).expect("lowers");
        for p in &mut n.block_programs {
            p.pad = rerail_compiler::native_cpu_block::BACK as u16;
        }
        let p = json_parser(&n);
        // `RERAIL_JSON_PROGRAM=<path>`: the block program as text (its operations, outputs and
        // records), to read what each word of the masks is.
        if let Ok(path) = std::env::var("RERAIL_JSON_PROGRAM") {
            let mut text = String::new();
            rerail_compiler::block::block_rows(&mut text, 0, &json_split(Split::Tier2).block_programs[0]);
            std::fs::write(&path, text).expect("writes");
        }
        // `RERAIL_JSON_ASM=<path>`: tier 1's machine code, as the JIT made it.
        if let Ok(path) = std::env::var("RERAIL_JSON_ASM") {
            std::fs::write(&path, &p.tier1.disassembly).expect("writes");
        }
        // `RERAIL_JSON_CLIF=<path>`: each pass's Cranelift IR, as the JIT was given it.
        if let Ok(path) = std::env::var("RERAIL_JSON_CLIF") {
            std::fs::write(&path, p.tier1.passes.iter().map(|x| x.clif.as_str()).collect::<Vec<_>>().join("\n")).expect("writes");
        }
        if let Ok(path) = std::env::var("RERAIL_JSON_RUST") {
            let options = rerail_compiler::native_cpu_codegen::options();
            options.keep_ir.store(true, std::sync::atomic::Ordering::Relaxed);
            let bit = options.bit_test.swap(true, std::sync::atomic::Ordering::Relaxed);
            let kept = json_parser(&n);
            options.bit_test.store(bit, std::sync::atomic::Ordering::Relaxed);
            let src = rerail_compiler::clif_rust::rust_source(&kept).unwrap_or_else(|e| panic!("to Rust: {e}"));
            std::fs::write(&path, &src).expect("writes");
            eprintln!("tier 1 as Rust: {path}, {} bytes", src.len());
        }
        // `RERAIL_JSON_ONLY=<name>`: that made input alone, no corpus (to trace one input's path).
        let only = std::env::var("RERAIL_JSON_ONLY").ok();
        let files: Vec<Vec<u8>> = if only.is_some() { Vec::new() } else { json_files().into_iter().filter(|f| p.image(f).is_ok()).collect() };
        let bytes: usize = files.iter().map(Vec::len).sum::<usize>().max(1);
        let passes = p.tier1.pass_count();
        let mut batches: Vec<_> = files.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_slice()])).collect();
        for b in &mut batches {
            b.call(&p.tier1);
        }
        let mut best = vec![(u64::MAX, u64::MAX); passes + 1];
        let kernel = rerail_simdjson_native::Kernel::best().expect("a simdjson kernel");
        for _ in 0..rounds {
            for k in 0..passes {
                let (i0, c0) = retired();
                for b in &mut batches {
                    b.call_pass(&p.tier1, k);
                }
                let (i1, c1) = retired();
                best[k] = (best[k].0.min(i1 - i0), best[k].1.min(c1 - c0));
            }
            let (i0, c0) = retired();
            let mut n = 0i64;
            for f in &files {
                n += kernel.stage1(f, false);
            }
            std::hint::black_box(n);
            let (i1, c1) = retired();
            best[passes] = (best[passes].0.min(i1 - i0), best[passes].1.min(c1 - c0));
        }
        eprintln!("{} files, {bytes} bytes; least of {rounds} rounds; simdjson's kernel {}", files.len(), kernel.name());
        let per = |x: u64| x as f64 / bytes as f64;
        for (k, &(i, c)) in best.iter().enumerate() {
            let name = if k == passes { "simdjson stage 1".to_string() } else { format!("tier 1 pass {k}{}", if k + 1 == passes { " (records)" } else { "" }) };
            eprintln!("{name:>24}: {:7.3} instructions {:6.3} cycles a byte", per(i), per(c));
        }
        // Inputs that part the block's masks from the token loop: strings of 5,000 bytes (a few
        // tokens: the masks alone), the same of two-byte UTF-8, white space, and dense tokens.
        // Each input is cut into files under the tape's 16-bit widths: 16 files of about 60 KB.
        let n = 60_000;
        // Numbers of 1, 4 and 12 digits part the token loop's cost a token from its cost a byte.
        let synth: [(&str, Vec<u8>); 7] = [
            ("long ASCII strings", [b"[".as_slice(), &format!("\"{}\",", "a".repeat(5_000)).repeat(n / 5_000).into_bytes(), b"0]"].concat()),
            ("long UTF-8 strings", [b"[".as_slice(), &format!("\"{}\",", "\u{e9}".repeat(2_500)).repeat(n / 5_000).into_bytes(), b"0]"].concat()),
            ("white space", [b"[".as_slice(), &vec![b' '; n], b"]"].concat()),
            ("numbers", [b"[".as_slice(), &"1234,".repeat(n / 5).into_bytes(), b"0]"].concat()),
            ("one-digit numbers", [b"[".as_slice(), &"1,".repeat(n / 2).into_bytes(), b"0]"].concat()),
            ("12-digit numbers", [b"[".as_slice(), &"123456789012,".repeat(n / 13).into_bytes(), b"0]"].concat()),
            ("short strings", [b"[".as_slice(), &"\"ab\",".repeat(n / 5).into_bytes(), b"0]"].concat()),
        ];
        for (name, f) in synth.iter().filter(|(name, _)| only.as_deref().is_none_or(|o| o == *name)) {
            p.image(f).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let mut bs: Vec<_> = (0..16).map(|_| rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_slice()])).collect();
            for b in &mut bs {
                b.call(&p.tier1);
            }
            let mut best = (u64::MAX, u64::MAX);
            for _ in 0..rounds {
                let (i0, c0) = retired();
                for b in &mut bs {
                    b.call_pass(&p.tier1, passes - 1);
                }
                let (i1, c1) = retired();
                best = (best.0.min(i1 - i0), best.1.min(c1 - c0));
            }
            let mut s = (u64::MAX, u64::MAX);
            for _ in 0..rounds {
                let (i0, c0) = retired();
                for _ in 0..16 {
                    std::hint::black_box(kernel.stage1(f, false));
                }
                let (i1, c1) = retired();
                s = (s.0.min(i1 - i0), s.1.min(c1 - c0));
            }
            let per = |x: u64| x as f64 / (16 * f.len()) as f64;
            eprintln!("{name:>24}: tier 1 {:7.3} instructions {:6.3} cycles a byte; simdjson {:6.3} {:6.3}", per(best.0), per(best.1), per(s.0), per(s.1));
        }
    }

    /// JSON's program with its strings split where `at` says (decision 0323).
    fn json_split(at: rerail_compiler::records::Split) -> rerail_compiler::program::Words {
        // `RERAIL_JSON_BITLEX` names another grammar file, for a probe's variants.
        let text = std::env::var("RERAIL_JSON_BITLEX").map(|p| std::fs::read_to_string(p).expect("the grammar")).unwrap_or_else(|_| include_str!("../../json_parser/json.bitlex").to_owned());
        let mut w = program(&text).unwrap_or_else(|e| panic!("json.bitlex: {e}"));
        w.block_programs[0].records.as_mut().expect("records").split_parts_in(at);
        w
    }

    /// Lowering B (decision 0323): tier 1 reads a string whole and tier 2's `TOKEN` splits it.
    /// The nodes are A's: by the interpreter both ways, and by Cranelift's tier 2 over the CPU's
    /// tier 1 under B, and over A's records (split already, as Metal's masks give them).
    #[test]
    fn jsons_tier2_split_is_tier1s() {
        use rerail_compiler::records::Split;
        let (a, b) = (json_split(Split::Tier1), json_split(Split::Tier2));
        let lower = |w: &rerail_compiler::program::Words| {
            let mut n = rerail_compiler::native_ir::lower(w, 0, 0).expect("lowers");
            for p in &mut n.block_programs {
                p.pad = rerail_compiler::native_cpu_block::BACK as u16;
            }
            n
        };
        let (na, nb) = (lower(&a), lower(&b));
        let (t1a, t1b) = (rerail_compiler::native_cpu_block::compile(&na.block_programs[0]).expect("A's tier 1"), rerail_compiler::native_cpu_block::compile(&nb.block_programs[0]).expect("B's tier 1"));
        let t2b = rerail_compiler::native_cpu_stream::compile(&nb).expect("B's tier 2");
        let t2a = rerail_compiler::native_cpu_stream::compile(&na).expect("A's tier 2");
        // And tier 2 a function a grammar rule (`by_rules`, rr's), its `TOKEN` a function of
        // its own (`take_token`): the tapes, A's and B's.
        let (pa, pb) = (json_parser(&na), json_parser(&nb));
        let mut inputs: Vec<Vec<u8>> = vec![
            br#"{"a": [1, -2.5e3, true, null], "b\n": {"c": "dAx"}}"#.to_vec(),
            br#"["", "\\", "a\"b", "\\\"", "x", "\n\t", "true"]"#.to_vec(),
            br#""open"#.to_vec(),
            br#"["\q"]"#.to_vec(),
        ];
        for pad in [0usize, 1, 61, 62, 63, 64, 65, 127] {
            for n in [0usize, 1, 62, 63, 64, 130] {
                inputs.push(format!("{}[\"{}\\n{}\", \"\\\\\\\"{}\"]", " ".repeat(pad), "s".repeat(n), "t".repeat(n), "u".repeat(n)).into_bytes());
            }
        }
        inputs.extend(json_files());
        let nodes = |r: Result<Vec<rerail_compiler::stream::Node>, rerail_compiler::stream::Refused>| r.ok().map(|n| n.iter().map(|x| (x.kind, x.start, x.end, x.size)).collect::<Vec<_>>());
        let (mut parsed, mut strings, mut wide) = (0usize, 0usize, 0usize);
        let se = usize::from(a.block_programs[0].records.as_ref().expect("records").starts);
        for (k, f) in inputs.iter().enumerate() {
            let want = nodes(rerail_compiler::stream::run(&a, f, &a.block_programs[0].run(f)));
            let by_b = nodes(rerail_compiler::stream::run(&b, f, &b.block_programs[0].run(f)));
            let show = || String::from_utf8_lossy(&f[..f.len().min(100)]).into_owned();
            assert_eq!(by_b, want, "file {k} ({:?}): the interpreter under B", show());
            let (ra, rb) = (t1a.run_files(&[f])[0].clone(), t1b.run_files(&[f])[0].clone());
            let refused = |t: &[u64]| t.iter().any(|&x| x != 0);
            assert_eq!(refused(&rb.totals), refused(&ra.totals), "file {k}: B's tier 1 refusal");
            if refused(&ra.totals) {
                continue;
            }
            assert!(rb.positions[se].len() <= ra.positions[se].len(), "file {k}: B's tier 1 has a record a string");
            strings += ra.positions[se].len() - rb.positions[se].len();
            let why = |r: &Result<Vec<rerail_compiler::stream::Node>, rerail_compiler::stream::Refused>| r.as_ref().err().map(|e| (e.pc, e.why));
            let (gb, ga) = (t2b.run(f, &rb), t2b.run(f, &ra));
            // A's own tier 2 refuses what its tape's widths cannot hold (a size past 16 bits):
            // B's then refuses it too.
            let gaa = t2a.run(f, &ra);
            if gaa.is_ok() {
                assert_eq!(nodes(gaa.clone()), want, "file {k} ({:?}): A's Cranelift tier 2 over A's records", show());
            } else if want.is_some() {
                wide += 1;
                let most = want.iter().flatten().map(|n| n.3).max().unwrap_or(0);
                assert!(most > 0xffff || f.len() > 0xffff, "file {k} ({:?}, {} bytes): A's Cranelift tier 2 refuses ({:?}), its tape's widths would hold it", show(), f.len(), why(&gaa));
            }
            let want = if gaa.is_ok() { want } else { None };
            assert_eq!(pb.image(f).ok(), pa.image(f).ok(), "file {k} ({:?}): the tapes by rules, B's and A's", show());
            assert_eq!(nodes(gb.clone()), want, "file {k} ({:?}): Cranelift's tier 2 over B's tier 1 ({:?}; over A's records {:?})", show(), why(&gb), why(&ga));
            assert_eq!(nodes(ga.clone()), want, "file {k} ({:?}): Cranelift's tier 2 over A's records ({:?})", show(), why(&ga));
            parsed += 1;
        }
        eprintln!("{} files, {parsed} parsed by both tier 1s ({wide} past the tape's widths); B's tier 1 wrote {strings} records fewer", inputs.len());
    }

    /// A region read in parts on the CPU (decision 0323, lowering A): tier 1's token loop
    /// writes a string's quote, contents, escapes and closer as the interpreter's records
    /// (`records::read_parts`), with their codes, on strings with escapes at every offset of a
    /// block's edge, over blocks, open at the file's end, and on JSON's files.
    #[test]
    fn jsons_cpu_records_are_the_interpreters() {
        let program = program(include_str!("../../json_parser/json.bitlex")).unwrap_or_else(|e| panic!("json.bitlex: {e}"));
        let block = &program.block_programs[0];
        let r = block.records.as_ref().expect("records");
        assert!(r.readers.iter().any(|rd| !rd.parts.is_empty()), "a reader in parts");
        let mut native = rerail_compiler::native_ir::lower(&program, 0, 0).expect("lowers");
        for b in &mut native.block_programs {
            b.pad = rerail_compiler::native_cpu_block::BACK as u16;
        }
        let cpu = rerail_compiler::native_cpu_block::compile(&native.block_programs[0]).expect("the token loop compiles");
        let mut inputs: Vec<Vec<u8>> = [
            &br#"{"a": [1, -2.5e3, true, null], "b\n": {"c": "dAx"}}"#[..],
            br#""""#,
            br#""\\""#,
            br#"["a\"b", "\\\"", "x"]"#,
            br#"["\\\\", "\"\"", "a\\", "\\a", "true", "\n\t\r"]"#,
            br#"{"k":"v"}"#,
            br#""open"#,
            br#""open\"#,
            br#"["\q"]"#,
        ].iter().map(|x| x.to_vec()).collect();
        for pad in [0usize, 1, 30, 58, 59, 60, 61, 62, 63, 64, 65, 126, 127] {
            for t in [r#""\n""#, r#""ab\"cd""#, r#""x\\""#, r#""AB""#, r#""\\\\\\\"""#, r#""true""#, r#"["a",1,"\"",{"b":"c\nd"}]"#] {
                inputs.push(format!("{}{t}", " ".repeat(pad)).into_bytes());
                inputs.push(format!("[{}{t}, 1]", "1,".repeat(pad / 2)).into_bytes());
            }
            // A string over blocks, its escapes on each side of their edges.
            for n in [60usize, 63, 64, 65, 130, 200] {
                inputs.push(format!("{}\"{}\\n{}\"", " ".repeat(pad), "s".repeat(n), "t".repeat(n)).into_bytes());
                inputs.push(format!("{}[\"{}\", \"{}\\\\\"]", " ".repeat(pad), "s".repeat(n), "\\\"".repeat(n / 2)).into_bytes());
                inputs.push(format!("{}\"{}", " ".repeat(pad), "s".repeat(n)).into_bytes());
            }
        }
        // A byte order mark, whole or cut, at the start and not (an `early`, decision 0330), in
        // short files and before a value over a block's edge.
        for head in [&b"\xef\xbb\xbf"[..], b"\xef\xbb", b"\xef", b" \xef\xbb\xbf", b"\xef\xbb\xbf\xef\xbb\xbf"] {
            for t in ["", "1", "[1]", "\"a\"", "  {\"k\": 12}", &*format!("[{}0]", "1,".repeat(40))] {
                inputs.push([head, t.as_bytes()].concat());
            }
        }
        let files = json_files();
        inputs.extend(files);
        let refs: Vec<&[u8]> = inputs.iter().map(|f| f.as_slice()).collect();
        let got = cpu.run_files(&refs);
        let refusal = |t: &[u64]| t.iter().any(|&x| x != 0);
        let (mut strings, mut parts) = (0usize, 0usize);
        for (k, (f, g)) in refs.iter().zip(&got).enumerate() {
            let want = block.run(f);
            let refused = refusal(&want.totals);
            assert_eq!(refusal(&g.totals), refused, "file {k} ({:?}): the refusal", String::from_utf8_lossy(&f[..f.len().min(80)]));
            let upto = |x: usize, y: usize| if refused { x.min(y) } else { x.max(y) };
            let (gs, gl) = (&g.positions[usize::from(r.starts)], &g.positions[usize::from(r.lasts)]);
            let (ws, wl) = (&want.positions[usize::from(r.starts)], &want.positions[usize::from(r.lasts)]);
            let n = upto(gs.len(), ws.len());
            if let Some(i) = (0..n).find(|&i| gs.get(i) != ws.get(i) || gl.get(i) != wl.get(i) || g.codes.get(i) != want.codes.get(i)) {
                let around = |s: &Vec<u32>, l: &Vec<u32>, c: &Vec<u32>| (i.saturating_sub(2)..(i + 3).min(s.len())).map(|j| (s[j], l.get(j).copied(), c.get(j).map(|x| format!("{x:#x}")))).collect::<Vec<_>>();
                panic!("file {k} ({:?}): token {i}: the CPU's (start, last, code) {:?}, the interpreter's {:?}", String::from_utf8_lossy(&f[..f.len().min(120)]), around(gs, gl, &g.codes), around(ws, wl, &want.codes));
            }
            if !refused {
                assert_eq!(g.totals, want.totals, "file {k}: the totals");
                strings += f.iter().filter(|&&b| b == b'"').count();
                parts += ws.len();
            }
        }
        eprintln!("{} files, {strings} quotes, {parts} tokens the same", refs.len());
    }
}
