//! The Bitlex compiler (`bitlex_compile`), checked against the reference evaluator
//! (`tools/bitlex`): each named mask of the compiled block program equals the evaluator's mask on
//! every input and every executor (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).

use super::block_tests::{Executors, executors, inputs};
use super::*;

/// The masks the compiler handles so far: classes, logic, `prev`, `starts`, `after`, `span`,
/// `toggle`, `after_odd_run` and `fill`.
const FORWARD: &str = r#"
lexer forward
class quote     = '"'
class backslash = '\\'
class ws        = ' ' '\t' '\n' '\r' 0x0b 0x0c
class letter    = 'a'..'z' 'A'..'Z'
class high      = 0x80..0xff
class scalar    = '-' '+' '.' '0'..'9' 'a'..'z' 'A'..'Z'
class none      = 0x00
mask escaped    = after_odd_run(backslash)
mask q          = quote & ~escaped
mask inside     = toggle(q)
mask code       = ~inside & ~q
mask word       = code & scalar
mask wstart     = starts(word)
mask gap        = after(ws)
mask esc_bytes  = span(backslash & inside, 2)
mask mixed      = (letter ^ high) | prev(quote, 3) | prev(none, 63) | empty
mask lit        = fill(wstart & letter, ~word)
mask after_dot  = fill(prev(quote) & word, ~word)
"#;

const NAMES: [&str; 15] = [
    "quote", "backslash", "ws", "letter", "high", "scalar", "none", "escaped", "q", "inside", "code", "word", "wstart",
    "gap", "esc_bytes",
];

/// The evaluator's words for `name`, and the executor's cut to the same `n` bits.
fn evaluated(stmts: &[::bitlex::syntax::Stmt], bytes: &[u8], names: &[&str]) -> Vec<Vec<u64>> {
    let tables = std::collections::HashMap::new();
    let out = ::bitlex::eval::Ev::run(stmts, bytes, &tables).unwrap_or_else(|e| panic!("evaluates: {e}"));
    names
        .iter()
        .map(|name| match out.vars.get(*name) {
            Some(::bitlex::eval::Val::M(m)) => m.w.clone(),
            _ => panic!("{name} is not a mask"),
        })
        .collect()
}

fn cut(mut words: Vec<u64>, n: usize) -> Vec<u64> {
    words.truncate(n.div_ceil(64));
    if n % 64 != 0 {
        if let Some(last) = words.last_mut() {
            *last &= (1u64 << (n % 64)) - 1;
        }
    }
    words
}

#[test]
fn forward_masks_equal_the_evaluator_on_every_executor() {
    let mut names: Vec<&str> = NAMES.to_vec();
    names.extend(["mixed", "lit", "after_dot"]);
    let stmts = ::bitlex::syntax::parse(FORWARD).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names, None).unwrap_or_else(|e| panic!("compiles: {e}"));
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let mut cases = inputs();
    cases.push(br#"{"a\"b": [1, -2.5e3, "x\\"], "c": true}"#.to_vec());
    for bytes in cases {
        let want = evaluated(&stmts, &bytes, &names);
        for (executor, got) in executors(&p, &bytes) {
            for (k, name) in names.iter().enumerate() {
                assert_eq!(cut(got[k].clone(), bytes.len()), want[k], "{name}, {executor}, {} bytes\n{}", bytes.len(), compiled.rows);
            }
        }
    }
}

/// Classes pack into nibble tables as unions of products, shared between classes: letters are
/// two products (high nibbles 4 and 6 with low nibbles 1 to 15; 5 and 7 with 0 to 10), and
/// `scalar` reuses them and adds two (`-`, the digits). A one-byte class is an `eq`.
#[test]
fn classes_pack_into_shared_products() {
    let src = "class letter = 'a'..'z' 'A'..'Z'\nclass scalar = '-' '0'..'9' 'a'..'z' 'A'..'Z'\nclass quote = '\"'\n";
    let stmts = ::bitlex::syntax::parse(src).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &["letter", "scalar", "quote"], None).expect("compiles");
    let tables = compiled.rows.lines().filter(|l| l.contains(" table ")).count();
    // Four products: one table.
    assert_eq!(tables, 1, "{}", compiled.rows);
    assert!(compiled.rows.contains("= eq 0x22"), "{}", compiled.rows);
}

/// What the compiler does not handle yet is refused by name, not guessed.
#[test]
fn unsupported_operations_are_refused() {
    let stmts = ::bitlex::syntax::parse("class a = 'a'\nmask b = text(\"ab\") & a\n").expect("parses");
    let error = bitlex_compile::compile(&stmts, &["b"], None).expect_err("refused");
    assert!(error.contains("text"), "{error}");
}

/// Errors and escapes are the fallback output: on every input and every executor its bits equal
/// the OR of their masks, and a file has one exactly when the evaluator reports one.
const FALLBACKS: &str = r#"
class quote     = '"'
class backslash = '\\'
class slash     = '/'
class star      = '*'
class lf        = '\n'
mask escaped    = after_odd_run(backslash)
mask inside     = toggle(quote & ~escaped)
mask code       = ~inside & ~quote
error "newline in a string"  when inside & lf
escape "a comment"           when code & (slash | star)
"#;

#[test]
fn errors_and_escapes_are_the_fallback_output() {
    let names = ["inside", "code", bitlex_compile::FALLBACK];
    let stmts = ::bitlex::syntax::parse(FALLBACKS).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names, None).unwrap_or_else(|e| panic!("compiles: {e}"));
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let mut cases = inputs();
    cases.extend([b"{\"a/b\": 1}".to_vec(), b"{\"a\": 1} // c".to_vec(), b"\"a\nb\"".to_vec(), b"/".to_vec()]);
    let (mut clean, mut falls) = (0, 0);
    for bytes in cases {
        let tables = std::collections::HashMap::new();
        let out = ::bitlex::eval::Ev::run(&stmts, &bytes, &tables).expect("evaluates");
        let vars = |name: &str| match out.vars.get(name) {
            Some(::bitlex::eval::Val::M(m)) => m.clone(),
            _ => panic!("{name}"),
        };
        let (inside, code, lf) = (vars("inside"), vars("code"), vars("lf"));
        let (slash, star) = (vars("slash"), vars("star"));
        let want = inside.and(&lf).or(&code.and(&slash.or(&star))).w;
        // The program has no tokens, so the evaluator's "in no token" errors are not its own.
        let falls_back = out.errors.iter().any(|e| !e.contains("is in no token"));
        assert_eq!(want.iter().any(|&w| w != 0), falls_back, "{} bytes: {:?}", bytes.len(), out.errors);
        for (executor, got) in executors(&p, &bytes) {
            // The fallback's outputs, one per delay, ORed: the OR of the masks.
            let mut any = vec![0u64; bytes.len().div_ceil(64)];
            for &k in &compiled.fallbacks {
                for (a, w) in any.iter_mut().zip(cut(got[usize::from(k)].clone(), bytes.len())) {
                    *a |= w;
                }
            }
            assert_eq!(any, want, "{executor}, {} bytes", bytes.len());
        }
        if falls_back { falls += 1 } else { clean += 1 }
    }
    assert!(clean > 0 && falls > 0, "{clean} clean, {falls} falling back");
}

/// Masks that read later bytes, by delays: `next`, `lasts`, the last byte, `next` of a value
/// the zero padding passes (`eq 0x00`) and of a carried one, `next` inside `prev`, and
/// `aligned`; JSON's number checks among them.
const LATER: &str = r#"
class digit     = '0'..'9'
class sign      = '+' '-'
class expc      = 'e' 'E'
class nul       = 0x00
class quote     = '"'
class backslash = '\\'
mask a          = next(digit)
mask b          = next(sign) & next(digit, 2)
mask c          = lasts(digit)
mask last_byte  = ~next(~empty)
mask d          = next(nul) | next(nul, 5)
mask e          = next(toggle(quote))
mask f          = aligned(backslash, 2)
mask g          = aligned(digit, 4) ^ aligned(quote, 8)
mask h          = expc & (~prev(digit) | ~(next(digit) | (next(sign) & next(digit, 2))))
mask i          = prev(next(digit, 3), 2) & ~nul
mask j          = fill(next(quote) & digit, ~digit)
"#;

#[test]
fn later_masks_equal_the_evaluator_on_every_executor() {
    let names = ["a", "b", "c", "last_byte", "d", "e", "f", "g", "h", "i", "j"];
    let stmts = ::bitlex::syntax::parse(LATER).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names, None).unwrap_or_else(|e| panic!("compiles: {e}"));
    assert!(compiled.rows.contains("delay 5"), "the largest next is 5 bytes\n{}", compiled.rows);
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let mut cases = inputs();
    cases.extend([b"1e+5 2E-x 3e 4e9 \"\\\\\\\"\0".to_vec(), vec![0u8; 70], b"\"\"\"".to_vec()]);
    for bytes in cases {
        let want = evaluated(&stmts, &bytes, &names);
        for (executor, got) in executors(&p, &bytes) {
            for (k, name) in names.iter().enumerate() {
                assert_eq!(cut(got[k].clone(), bytes.len()), want[k], "{name}, {executor}, {} bytes\n{}", bytes.len(), compiled.rows);
            }
        }
    }
}

/// The whole of `json.bitlex`: on every input and every executor, a file the evaluator rejects
/// falls back, and for one that neither rejects, each row's masks equal the evaluator's: the
/// tree section's starts, ends and facts, and each token kind's starts and last bytes.
const JSON: &str = include_str!("../../../../../docs/parser-generators/lexers/json.bitlex");

const JSON_CASES: [&[u8]; 12] = [
    br#"{"a": 1, "b": [true, false, null], "c": {"d": "x\"y\\z", "e": -0.5e+10}}"#,
    b"[1, 2, [3, [4, {\"k\": []}]], {}]",
    b"  \n{\"key\":\"value\"}\n",
    b"\"just a string \\u00e9\"",
    b"42",
    br#"{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":{"j":{"k":{"l":{"m":1}}}}}}}}}}}}}"#,
    br#"[[[[[[[[[[[[[[1]]]]]]]]]]]]]]"#,
    b"[tru, nul]",
    b"{\"a\": 01}",
    b"}{",
    b"{\"a\": \"b",
    b"[1 // c\n]",
];

#[test]
fn json_rows_equal_the_evaluator_on_every_executor() {
    use ast_grep_language::LanguageExt;
    let grammar = crate::code_rule_dsl::language::GrammarDialect::Json.get_ts_language();
    let kind = |name: &str| {
        let id = match grammar.id_for_node_kind(name, true) {
            0 => grammar.id_for_node_kind(name, false),
            id => id,
        };
        (id != 0).then_some(id)
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).unwrap_or_else(|e| panic!("compiles: {e}"));
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let ops = compiled.rows.lines().filter(|l| l.contains(" = ")).count();
    let carried = compiled.rows.lines().filter(|l| [" = fill ", " = toggle ", " = escaped ", " = prev "].iter().any(|op| l.contains(op))).count();
    eprintln!("{ops} operations, {carried} carried or prev, {} outputs", compiled.rows.lines().filter(|l| l.contains(" output ")).count());
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let prepared = Executors::new(&p);
    let tokens: Vec<&str> = stmts.iter().filter_map(|s| match s { ::bitlex::syntax::Stmt::Token { kind, .. } => Some(kind.as_str()), _ => None }).collect();
    // A token row's tokens: its kind's and its keywords' kinds (the evaluator retypes them).
    let members = |row: &str| -> Vec<String> {
        let mut out = vec![row.to_owned()];
        for s in &stmts {
            if let ::bitlex::syntax::Stmt::Keywords { kind, words, .. } = s
                && kind == row
            {
                out.extend(words.iter().map(|(w, alias)| alias.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned())));
            }
        }
        out
    };
    let mut cases = inputs();
    cases.extend(JSON_CASES.iter().map(|c| c.to_vec()));
    let (mut compared, mut rejected, mut conservative) = (0, 0, 0);
    for bytes in cases {
        let n = bytes.len();
        let tables = std::collections::HashMap::new();
        let out = ::bitlex::eval::Ev::run(&stmts, &bytes, &tables).unwrap_or_else(|e| panic!("evaluates: {e}"));
        let token_mask = |kind: &str, last: bool| {
            let mut w = vec![0u64; n.div_ceil(64)];
            let kinds = members(kind);
            for t in out.tokens.iter().filter(|t| kinds.contains(&t.kind)) {
                let i = if last { t.end - 1 } else { t.start };
                w[i / 64] |= 1 << (i % 64);
            }
            w
        };
        let want = |label: &str| -> Option<Vec<u64>> {
            let words: Vec<&str> = label.split(' ').collect();
            match words.as_slice() {
                // A token row's first and last bytes.
                ["token", k, side] => tokens.contains(k).then(|| token_mask(k, *side == "end")),
                _ => None,
            }
        };
        let (runs, over) = prepared.run(&bytes, true);
        assert!(over.is_empty(), "over the launch budget: {over:?}");
        for (executor, got) in runs {
            // Tier 1 refuses by a fallback bit. The evaluator's other refusals (tokens that make
            // no tree) are tier 2's, which this test does not run.
            let falls = compiled.fallbacks.iter().any(|&k| cut(got[usize::from(k)].clone(), n).iter().any(|&w| w != 0));
            if !out.errors.is_empty() {
                rejected += 1;
                continue;
            }
            if falls {
                conservative += 1;
                continue;
            }
            // Every token's last byte, of any kind.
            let mut ends = vec![0u64; n.div_ceil(64)];
            for t in &out.tokens {
                ends[(t.end - 1) / 64] |= 1 << ((t.end - 1) % 64);
            }
            for (label, k) in &compiled.labels {
                if let Some(want) = want(label) {
                    let got = cut(got[usize::from(*k)].clone(), n);
                    let at = format!("{label}, {executor}, {n} bytes: {:?}", String::from_utf8_lossy(&bytes));
                    if label.ends_with(" end") {
                        // A row's end is tier 1's last-byte mask, which the row's kinds may share
                        // (`atom` and `number` both end at `lasts(word)`): it holds each of its
                        // tokens' last bytes, and only tokens' last bytes.
                        assert!(got.iter().zip(&want).all(|(g, w)| w & !g == 0), "{at}: a last byte missing");
                        assert!(got.iter().zip(&ends).all(|(g, e)| g & !e == 0), "{at}: a bit at no token's last byte");
                    } else {
                        assert_eq!(got, cut(want, n), "{at}");
                    }
                }
            }
            compared += 1;
        }
    }
    eprintln!("{compared} compared, {rejected} rejected, {conservative} falling back where the evaluator does not");
    assert!(compared > 0 && rejected > 0);
}

/// All of `json.bitlex` as generated MSL: one kernel, a few lines per instruction (it was 0.7 to
/// 1.1 MB of round kernels, each recomputing what came before it, and took Metal minutes).
#[test]
#[cfg(target_os = "macos")]
fn json_msl_is_one_kernel() {
    use ast_grep_language::LanguageExt;
    use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_with};
    let grammar = crate::code_rule_dsl::language::GrammarDialect::Json.get_ts_language();
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).expect("compiles");
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).expect("assembles");
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    for width in LaneWidth::ALL {
        for strategy in ClassStrategy::ALL {
            let msl = generate_with(&native, width, strategy);
            assert_eq!(msl.matches("kernel void ").count(), 1);
            let lines = msl.lines().count();
            // A fixed preamble (the kernel's arguments, tables and stores), then a few lines per
            // instruction.
            assert!(lines < 200 + 10 * native.insts.len(), "{}-byte {strategy:?}: {lines} lines for {} instructions", width.bytes(), native.insts.len());
        }
    }
}

/// A batch (decision 0044): files of several languages, interleaved, each under its own program,
/// in one launch. Every file's outputs and positions equal its program's CPU interpreter on that
/// file alone, on the Metal interpreter and on generated MSL at every lane width. With one poll
/// per predecessor and none after it, groups leave unfinished and the launch is repeated; the
/// results are the same.
#[test]
#[cfg(target_os = "macos")]
fn a_batch_of_files_and_programs_is_one_launch() {
    use metal_program_backend::block_program::{ClassStrategy, Executor, LaneWidth, run_batch};
    let programs = batch_programs();
    let blocks: Vec<&block::BlockProgram> = programs.iter().map(|p| &p.block_programs()[0]).collect();
    let natives: Vec<native_ir::BlockNative> = programs.iter().map(|p| native_ir::lower(p).expect("lowers").block_programs.remove(0)).collect();
    let natives: Vec<&native_ir::BlockNative> = natives.iter().collect();
    let bytes = batch_bytes();
    let files = batch_files(&bytes, blocks.len());
    let want: Vec<_> = files.iter().map(|f| blocks[f.program].run(f.bytes)).collect();
    for width in LaneWidth::ALL {
        for executor in [Executor::Interpreter, Executor::Generated(ClassStrategy::Lookup), Executor::Generated(ClassStrategy::Planes)] {
            for spins in [64, 0] {
                let run = run_batch(&natives, &files, width, executor, spins).unwrap_or_else(|e| panic!("{executor:?}: {e:?}"));
                if spins > 0 {
                    assert_eq!(run.launches, 1, "{executor:?}, {}-byte lanes", width.bytes());
                }
                for (k, (got, want)) in run.files.iter().zip(&want).enumerate() {
                    let what = format!("file {k} ({} bytes, program {}), {executor:?}, {}-byte lanes, {spins} polls", files[k].bytes.len(), files[k].program, width.bytes());
                    assert_eq!(got.outputs, want.outputs, "{what}");
                    assert_eq!(got.positions, want.positions, "{what}");
                }
            }
        }
    }
}

/// The batch tests' programs: JSON's lexer and tree, three test lexers, simdjson's stage 1, and a
/// chain of carries.
fn batch_programs() -> Vec<Program> {
    use super::block_tests::STAGE1;
    use ast_grep_language::LanguageExt;
    let grammar = crate::code_rule_dsl::language::GrammarDialect::Json.get_ts_language();
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let compiled = |src: &str, names: &[&str], symbols: Option<&bitlex_compile::Symbols<'_>>| {
        let stmts = ::bitlex::syntax::parse(src).expect("parses");
        let rows = bitlex_compile::compile(&stmts, names, symbols).unwrap_or_else(|e| panic!("compiles: {e}")).rows;
        rir::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n")).expect("assembles")
    };
    let chain = "program -\n  mode set=0 query=0\n  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = escaped %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 %5 = prev %4, 5\n  block 0 %6 = toggle %5\n  block 0 output %4\n  block 0 output %6\n  0000 STOP 0\n";
    vec![
        compiled(JSON, &[bitlex_compile::FALLBACK], Some(&symbols)),
        compiled(LATER, &["a", "b", "c", "last_byte", "d", "e", "f", "g", "h", "i", "j"], None),
        compiled(FORWARD, &NAMES, None),
        rir::assemble(STAGE1).expect("assembles"),
        rir::assemble(chain).expect("assembles"),
    ]
}

/// The batch tests' files: lexer inputs, carry runs, JSON cases, and an empty file.
fn batch_bytes() -> Vec<Vec<u8>> {
    let mut bytes: Vec<Vec<u8>> = inputs().into_iter().step_by(7).collect();
    bytes.extend(super::block_tests::runs());
    bytes.extend(JSON_CASES.iter().map(|c| c.to_vec()));
    bytes.push(Vec::new());
    bytes
}

/// The files of a batch, interleaved over `programs` programs.
fn batch_files(bytes: &[Vec<u8>], programs: usize) -> Vec<block::BatchFile<'_>> {
    bytes.iter().enumerate().map(|(i, b)| block::BatchFile { bytes: b, program: (i * 3 + i / 5) % programs }).collect()
}

/// The CPU executors over the same batch: the interpreter (one dispatch per operation over all of
/// a program's files) and Cranelift (one call per program, over all its files). Each file's
/// results equal its program's interpreter on that file alone.
#[test]
fn the_cpu_executors_take_a_batch() {
    let programs = batch_programs();
    let blocks: Vec<&block::BlockProgram> = programs.iter().map(|p| &p.block_programs()[0]).collect();
    let compiled: Vec<_> = programs
        .iter()
        .map(|p| native_cpu_block::compile(&native_ir::lower(p).expect("lowers").block_programs.remove(0)).expect("compiles"))
        .collect();
    let compiled: Vec<&native_cpu_block::CompiledBlock> = compiled.iter().collect();
    let bytes = batch_bytes();
    let files = batch_files(&bytes, blocks.len());
    let want: Vec<_> = files.iter().map(|f| blocks[f.program].run(f.bytes)).collect();
    for (name, got) in [("interpreter", block::run_batch(&blocks, &files)), ("cranelift", native_cpu_block::run_batch(&compiled, &files))] {
        assert_eq!(got.len(), files.len());
        for (k, (got, want)) in got.iter().zip(&want).enumerate() {
            let what = format!("{name}: file {k} ({} bytes, program {})", files[k].bytes.len(), files[k].program);
            assert_eq!(got.outputs, want.outputs, "{what}");
            assert_eq!(got.positions, want.positions, "{what}");
        }
    }
}

/// The batch's cost per 64-byte block on real files: JSON's program (masks, tree rows and the
/// fallback) over every file listed in `RERAIL_JSON_FILES` (one path a line), in one batch.
/// Generated MSL and the Metal interpreter at every lane width (GPU seconds, best of 3), and
/// Cranelift's one call per program (thread CPU time, best of 3). Run by name: a measurement.
#[test]
#[ignore = "a measurement; run by name with RERAIL_JSON_FILES"]
fn json_batch_cost_per_block() {
    let list = std::env::var("RERAIL_JSON_FILES").expect("RERAIL_JSON_FILES: a file of paths");
    let bytes: Vec<Vec<u8>> = std::fs::read_to_string(&list)
        .expect("reads the list")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|path| std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .collect();
    let program = batch_programs().remove(0);
    let native = native_ir::lower(&program).expect("lowers").block_programs.remove(0);
    let files: Vec<block::BatchFile<'_>> = bytes.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
    let total: usize = bytes.iter().map(Vec::len).sum();
    let blocks: usize = bytes.iter().map(|b| b.len().div_ceil(64).max(1)).sum();
    println!("{} files, {total} bytes, {blocks} blocks; {} instructions", files.len(), native.insts.len());
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let slices: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
    let runs = native_cpu_block::run_batch(&[&compiled], &files);
    let falls = json_fallbacks();
    let fallback = runs.iter().filter(|r| falls.iter().any(|&k| r.outputs[usize::from(k)].iter().any(|&w| w != 0))).count();
    println!("files to the fallback: {fallback}");
    let s = compiled.batch_cpu_seconds(&slices, 3);
    println!("cranelift, one call: {:.3} ns per block, {:.2} GB/s (CPU time, best of 3)", s / blocks as f64 * 1e9, total as f64 / s / 1e9);
    #[cfg(target_os = "macos")]
    if std::env::var_os("RERAIL_CPU_ONLY").is_none() {
        use metal_program_backend::block_program::{ClassStrategy, Executor, LaneWidth, REPEATED_LAUNCHES, run_split};
        let before = REPEATED_LAUNCHES.load(std::sync::atomic::Ordering::Relaxed);
        for executor in [Executor::Generated(ClassStrategy::Lookup), Executor::Generated(ClassStrategy::Planes), Executor::Interpreter] {
            for width in LaneWidth::ALL {
                let (mut best, mut launches) = (f64::MAX, 0);
                for _ in 0..3 {
                    let run = run_split(&[&native], &files, width, executor, 64).unwrap_or_else(|e| panic!("{executor:?}: {e:?}"));
                    assert!(run.files == runs, "{executor:?} equals Cranelift");
                    best = best.min(run.seconds);
                    launches = run.launches;
                }
                println!(
                    "{executor:?}, {:2}-byte lanes, {launches} launches: {:.3} ns per block, {:.2} GB/s (GPU time, best of 3)",
                    width.bytes(),
                    best / blocks as f64 * 1e9,
                    total as f64 / best / 1e9
                );
            }
        }
        println!("repeated launches: {}", REPEATED_LAUNCHES.load(std::sync::atomic::Ordering::Relaxed) - before);
    }
}

/// JSON's program.
fn json_program() -> Program {
    batch_programs().remove(0)
}

/// JSON's block program (tier 2's analysis reads it: `block_plan_tests`).
pub(super) fn json_block_program() -> block::BlockProgram {
    json_program().block_programs()[0].clone()
}

/// A JSON-grammar Bitlex text's block program: `json.bitlex` with a change, say.
pub(super) fn json_variant_block_program(src: &str) -> block::BlockProgram {
    use ast_grep_language::LanguageExt;
    let grammar = crate::code_rule_dsl::language::GrammarDialect::Json.get_ts_language();
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(src).expect("parses");
    let rows = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).unwrap_or_else(|e| panic!("compiles: {e}")).rows;
    rir::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n")).expect("assembles").block_programs()[0].clone()
}

/// `json.bitlex`'s text.
pub(super) const JSON_BITLEX: &str = JSON;

/// The one-byte kinds are the punctuation token's keywords, which tier 2 reads from the token's
/// byte: they are not tier 1's outputs, so tier 1 stays simdjson's stage 1 (its ratchets hold).
#[test]
fn json_punctuation_is_one_token_row() {
    let p = json_program();
    let b = &p.block_programs()[0];
    let one_byte: Vec<&[u8]> = b.keywords.iter().filter(|k| k.word.len() == 1).map(|k| &k.word[..]).collect();
    assert_eq!(one_byte, [&b"{"[..], b"}", b"[", b"]", b",", b":"]);
    let rows: std::collections::BTreeSet<u16> = b.keywords.iter().filter(|k| k.word.len() == 1).map(|k| k.node).collect();
    assert_eq!(rows.len(), 1, "one token row: the punctuation");
}

/// JSON's fallback outputs (`bitlex_compile::Compiled::fallbacks`).
fn json_fallbacks() -> Vec<u16> {
    use ast_grep_language::LanguageExt;
    let grammar = crate::code_rule_dsl::language::GrammarDialect::Json.get_ts_language();
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).expect("compiles").fallbacks
}


// --- The shape of a compiled language (decision 0047) -----------------------------------------
//
// A Bitlex file compiles to simdjson's shape: straight-line bit operations, a carry only where the
// language needs one. These rules hold for every compiled program; JSON's budgets are ratchets,
// lowered as the compiler improves, never raised.

/// A block program's violations of the shape rules, each a short line.
fn shape_violations(p: &block::BlockProgram) -> Vec<String> {
    use super::block::BlockOp::*;
    let ops = &p.ops;
    // Two byte classes that one class replaces: an `eq` with another, or two of one table. (A
    // class whose products span tables is one lookup per table, ORed.)
    let one_class = |a: u16, b: u16| match (ops[usize::from(a)], ops[usize::from(b)]) {
        (Eq(_), Eq(_) | Class { .. }) | (Class { .. }, Eq(_)) => true,
        (Class { table: x, .. }, Class { table: y, .. }) => x == y,
        _ => false,
    };
    let mut out = Vec::new();
    for (k, &v) in p.outputs.iter().enumerate() {
        // An output `prev(x, k)` is `x` output `k` bytes later.
        if matches!(ops[usize::from(v)], Prev(..)) {
            out.push(format!("output {k} is a prev (an alignment)"));
        }
    }
    for (at, op) in ops.iter().enumerate() {
        match *op {
            // `within` at a delay is a leaf, from the byte position.
            Prev(a, _) if matches!(ops[usize::from(a)], Within(_)) => out.push(format!("%{at}: a prev of within")),
            // `prev(prev(x, a), b)` is `prev(x, a + b)`, on the same carried word.
            Prev(a, _) if matches!(ops[usize::from(a)], Prev(..)) => out.push(format!("%{at}: a prev of a prev")),
            // Two byte classes combined are one class: one bitmask, not two and an operation.
            And(a, b) | Or(a, b) | Xor(a, b) | AndNot(a, b) if one_class(a, b) => out.push(format!("%{at}: two byte classes combined")),
            _ => {}
        }
    }
    out
}

/// JSON's program has simdjson's shape: no alignment carries, no `prev` of `within` or of a
/// `prev`, no two byte classes combined; and its operations and carried words within budget.
#[test]
fn json_compiles_to_simdjsons_shape() {
    let program = json_program();
    let p = &program.block_programs()[0];
    let violations = shape_violations(p);
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
    let prevs = p.ops.iter().filter(|op| matches!(op, block::BlockOp::Prev(..))).count();
    let (ops, words) = (p.ops.len(), super::block_tests::carried_words(p));
    eprintln!("json: {ops} operations, {prevs} prevs, {words} carried words, {} outputs", p.outputs.len());
    assert!(ops <= JSON_OPS && words <= JSON_CARRIED_WORDS, "json: {ops} operations (budget {JSON_OPS}), {words} carried words (budget {JSON_CARRIED_WORDS})");
}

/// JSON's carried state is bits (decision 0048): one per carried operation, and `k` per value a
/// `prev` looks back on. The information that crosses a block boundary is about 18 bits (4 of
/// lexical state: where in a string, number or literal, or between tokens; 14 of nesting: a
/// depth to 13 and whether each open bracket is '{'), plus the bits the rules look back on. A
/// ratchet, lowered as the compiler stops restating state.
#[test]
fn json_state_is_bits() {
    let program = json_program();
    let bits = super::block_tests::state_bits(&program.block_programs()[0]);
    eprintln!("json: {bits} state bits (budget {JSON_STATE_BITS})");
    assert!(bits <= JSON_STATE_BITS, "json: {bits} state bits (budget {JSON_STATE_BITS})");
}

/// JSON's state budget (a ratchet): 104 bits in one tier; 4 in two (decision 0049).
const JSON_STATE_BITS: usize = 4;

/// A token without `through` is its start byte alone, so its last byte is its start: no carry
/// finds it. JSON has two: the structural bytes (one row, kinds by text) and `"`.
#[test]
fn one_byte_tokens_end_at_their_start() {
    let program = json_program();
    let p = &program.block_programs()[0];
    let same = p.tokens.iter().filter(|n| n.start == n.end).count();
    assert_eq!(same, 2, "json's one-byte tokens' rows: {:?}", p.tokens.iter().map(|n| (n.kind, n.start, n.end)).collect::<Vec<_>>());
}

/// JSON's budgets (ratchets: lower them as the compiler improves).
const JSON_OPS: usize = 37;
const JSON_CARRIED_WORDS: usize = 4;

#[test]
#[ignore = "a dump; run by name"]
fn dump_json_rows() {
    std::fs::write(std::env::var("RERAIL_DUMP").expect("RERAIL_DUMP"), rir::disassemble(&json_program(), None)).expect("writes");
}

/// JSON's Cranelift loop: no vector `mvn`, one carried word per prev base and per carry, and its
/// stack loads and stores within budget (a ratchet, to reach 0 as the carried words fit).
#[test]
#[cfg(target_arch = "aarch64")]
fn json_loop_keeps_values_in_registers() {
    let program = json_program();
    let native = native_ir::lower(&program).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = super::block_tests::machine_loop(&compiled.disassembly);
    let (stack, mvn) = super::block_tests::loop_costs(&lines);
    eprintln!("json loop: {} instructions, {stack} stack loads or stores, {mvn} vector mvn", lines.len());
    assert_eq!(mvn, 0, "json's loop inverts vectors");
    // The state is packed: one carried word per 64 state bits (decision 0048).
    let bits = super::block_tests::state_bits(&program.block_programs()[0]);
    assert_eq!(super::block_tests::carried_params(&compiled.clif), bits.div_ceil(64) + native.extracts.len(), "json's carried state is not packed");
    assert!(lines.len() <= JSON_LOOP && stack <= JSON_LOOP_STACK, "json loop: {} instructions (budget {JSON_LOOP}), {stack} stack loads or stores (budget {JSON_LOOP_STACK})", lines.len());
}

/// JSON's loop budgets (ratchets): lower them as the program and the lowering improve. Packing
/// the carried state (decision 0048) raised the instructions once, 1915 -> 1939, for 405 -> 164
/// stack operations, at an equal or lower CPU time in every paired timing. Two tiers (decision
/// 0049) lowered them to 483 and 6, one structural class and kinds in tier 2 to 253 and 0, and a
/// full-block body with folded logic to 178, and one word token with the number check in tier 2
/// (decision 0052) to 174.
/// The target is stage 1's 150.
const JSON_LOOP: usize = 174;
const JSON_LOOP_STACK: usize = 0;

// --- Tier 1 is simdjson's stage 1 (decision 0049) ---------------------------------------------
//
// A language's block program (tier 1) does only the work that needs every byte: one
// classification, the string state, the token starts and the per-byte errors. Everything about
// a token (its kind, its text's checks, its place in the tree) is tier 2's, once per token.

/// A tier-1 program's violations: more string state than simdjson's (one `escaped`, one
/// `toggle`), a `fill` (a run is a token's; tier 2 reads it from the token's text), or a look-back
/// past the byte before (a token start looks one byte back, as simdjson's `follows`).
fn tier1_violations(p: &block::BlockProgram) -> Vec<String> {
    use super::block::BlockOp::*;
    let count = |f: fn(&block::BlockOp) -> bool| p.ops.iter().filter(|op| f(op)).count();
    let mut out = Vec::new();
    let runs = count(|op| matches!(op, Escaped(_)));
    if runs > 1 {
        out.push(format!("{runs} escapeds (simdjson: 1, the escapes)"));
    }
    let toggles = count(|op| matches!(op, Toggle(_)));
    if toggles > 1 {
        out.push(format!("{toggles} toggles (simdjson: 1, the strings)"));
    }
    let fills = count(|op| matches!(op, Fill(..)));
    if fills > 0 {
        out.push(format!("{fills} fills (a run is a token's: tier 2)"));
    }
    for (at, op) in p.ops.iter().enumerate() {
        if let Prev(_, k) = *op
            && k > 1
        {
            out.push(format!("%{at}: a look-back of {k} bytes (a token start looks 1 back)"));
        }
    }
    out
}

/// JSON's tier 1 is simdjson's stage 1: no more state, at most 4 carried bits, and a Cranelift
/// loop of at most 150 instructions with no stack traffic (simdjson's is about 100-150 per 64
/// bytes).
#[test]
#[cfg(target_arch = "aarch64")]
#[ignore = "red guard (decision 0049): on at step 3, when json.bitlex's dense part is stage 1's"]
fn json_tier1_is_simdjsons_stage1() {
    let program = json_program();
    let p = &program.block_programs()[0];
    let violations = tier1_violations(p);
    let bits = super::block_tests::state_bits(p);
    let native = native_ir::lower(&program).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = super::block_tests::machine_loop(&compiled.disassembly);
    let (stack, _) = super::block_tests::loop_costs(&lines);
    eprintln!("json tier 1: {} violations, {bits} state bits, loop {} instructions, {stack} stack", violations.len(), lines.len());
    if std::env::var_os("RERAIL_RIR").is_some() {
        eprintln!("{}", p.ops.iter().enumerate().map(|(i, op)| format!("%{i} = {}", op.text())).collect::<Vec<_>>().join("\n"));
        eprintln!("outputs {:?}\n{}", p.outputs, lines.join("\n"));
    }
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
    assert!(bits <= 4, "json tier 1: {bits} state bits (at most 4)");
    assert!(lines.len() <= 150 && stack == 0, "json tier 1 loop: {} instructions (at most 150), {stack} stack loads or stores (0)", lines.len());
}

/// JSON's tier 2 (doc 18 §3): the grammar's state machine, as `rir` text, pinned whole. Kinds:
/// 15 document, 17 object, 18 pair, 19 array, 20 string, 16 `_value`; tokens 1 `{`, 3 `}`,
/// 5 `[`, 6 `]`, 2 `,`, 4 `:`, 7 `"`, 8 string_content, 9 escape_sequence, 10 number, 11 true,
/// 12 false, 13 null; fields 1 key, 2 value. The states, by simdjson's stage-2 labels: 0 the
/// root's start, 2 `document_end`; 1 `object_begin`, 11 and 14 `object_field`, 15 and 18
/// `object_continue`, 17 after `,`; 3 `array_begin`, 12 `array_value`, 7 and 13
/// `array_continue`; 4, 5, 8, 16 `parse_string` (root, key, array, pair value); 10 after a root
/// string; 6 and 9 `scope_end`. A pair closes on the `,` or `}` after its value.
#[test]
fn json_tier2_states_print_as_designed() {
    let text = rir::disassemble(&json_program(), None);
    let got: String = text.lines().filter(|l| l.starts_with("  block 0 state ") || l.starts_with("  block 0 arm ")).map(|l| format!("{}\n", &l[10..])).collect();
    assert_eq!(got, JSON_STATES, "\n{got}");
}

const JSON_STATES: &str = "\
state 0 level 15\n\
state 1 level 17\n\
state 2 level 15 end 0\n\
state 3 level 19\n\
state 4 level 15\n\
state 5 level 17\n\
state 6 level 17 end 0\n\
state 7 level 19\n\
state 8 level 19\n\
state 9 level 19 end 0\n\
state 10 level 15 end 1\n\
state 11 level 17\n\
state 12 level 19\n\
state 13 level 19\n\
state 14 level 17\n\
state 15 level 17\n\
state 16 level 17\n\
state 17 level 17\n\
state 18 level 17\n\
arm 0 token 1 to 1 push 2 do open 15 open 17 s 16 leaf\n\
arm 0 token 5 to 3 push 2 do open 15 open 19 s 16 leaf\n\
arm 0 token 7 to 4 do open 15 open 20 s 16 leaf\n\
arm 0 token 10 to 2 do open 15 leaf s 16\n\
arm 0 token 11 to 2 do open 15 leaf s 16\n\
arm 0 token 12 to 2 do open 15 leaf s 16\n\
arm 0 token 13 to 2 do open 15 leaf s 16\n\
arm 1 token 7 to 5 do open 18 open 20 f 1 leaf\n\
arm 1 token 3 to 6 do leaf\n\
arm 3 token 1 to 1 push 7 do open 17 s 16 leaf\n\
arm 3 token 5 to 3 push 7 do open 19 s 16 leaf\n\
arm 3 token 7 to 8 do open 20 s 16 leaf\n\
arm 3 token 10 to 7 do leaf s 16\n\
arm 3 token 11 to 7 do leaf s 16\n\
arm 3 token 12 to 7 do leaf s 16\n\
arm 3 token 13 to 7 do leaf s 16\n\
arm 3 token 6 to 9 do leaf\n\
arm 4 token 8 to 4 do leaf\n\
arm 4 token 9 to 4 do leaf\n\
arm 4 token 7 to 10 do leaf\n\
arm 5 token 8 to 5 do leaf\n\
arm 5 token 9 to 5 do leaf\n\
arm 5 token 7 to 11 do leaf\n\
arm 7 token 2 to 12 do leaf\n\
arm 7 token 6 to 9 do leaf\n\
arm 8 token 8 to 8 do leaf\n\
arm 8 token 9 to 8 do leaf\n\
arm 8 token 7 to 13 do leaf\n\
arm 11 token 4 to 14 do close leaf\n\
arm 12 token 1 to 1 push 7 do open 17 s 16 leaf\n\
arm 12 token 5 to 3 push 7 do open 19 s 16 leaf\n\
arm 12 token 7 to 8 do open 20 s 16 leaf\n\
arm 12 token 10 to 7 do leaf s 16\n\
arm 12 token 11 to 7 do leaf s 16\n\
arm 12 token 12 to 7 do leaf s 16\n\
arm 12 token 13 to 7 do leaf s 16\n\
arm 13 token 2 to 12 do close leaf\n\
arm 13 token 6 to 9 do close leaf\n\
arm 14 token 1 to 1 push 15 do open 17 f 2 s 16 leaf\n\
arm 14 token 5 to 3 push 15 do open 19 f 2 s 16 leaf\n\
arm 14 token 7 to 16 do open 20 f 2 s 16 leaf\n\
arm 14 token 10 to 15 do leaf f 2 s 16\n\
arm 14 token 11 to 15 do leaf f 2 s 16\n\
arm 14 token 12 to 15 do leaf f 2 s 16\n\
arm 14 token 13 to 15 do leaf f 2 s 16\n\
arm 15 token 2 to 17 do close leaf\n\
arm 15 token 3 to 6 do close leaf\n\
arm 16 token 8 to 16 do leaf\n\
arm 16 token 9 to 16 do leaf\n\
arm 16 token 7 to 18 do leaf\n\
arm 17 token 7 to 5 do open 18 open 20 f 1 leaf\n\
arm 18 token 2 to 17 do close close leaf\n\
arm 18 token 3 to 6 do close close leaf\n\
";

/// Tier 2's states and arms are data (doc 18, decision 0051): the compiler pass makes them, `rir`
/// prints and assembles them, `validate` and the digest check them, and the lowering to the IR
/// (`native_ir`) reads them. Nothing walks them to parse a file: tier 2 runs as the IR, on the
/// IR's interpreter, Cranelift and Metal. A file outside this list that reads them fails here.
#[test]
fn only_the_compiler_reads_tier2s_states() {
    const ALLOWED: [&str; 7] = ["block.rs", "rir.rs", "rir_tests.rs", "grammar.rs", "bitlex_compile.rs", "bitlex_compile_tests.rs", "native_ir.rs"];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/transducer");
    // The table's types and its arms (`.states` is a common name; a state row is read through
    // its type).
    let needles = [".arms", "ArmRow", "StateRow", "CheckRow", "block::Act", "Act::Open", "Act::Close", "Act::Leaf"];
    let mut readers = Vec::new();
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("reads the source") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            if !(name.ends_with(".rs") || name.ends_with(".metal")) || ALLOWED.contains(&name.as_str()) {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("reads a source file");
            if needles.iter().any(|n| text.contains(n)) {
                readers.push(path.display().to_string());
            }
        }
    }
    assert!(readers.is_empty(), "tier 2's state table read outside the compiler: {readers:?}");
}

/// JSON's number check (doc 18 §4, decision 0052): simdjson's `parse_number` steps as
/// straight-line operations over the token's window. One `fill` per run (the integer, the
/// fraction, the exponent), shifts by one byte, and logic; no loop, no other carry. Its size is a
/// ratchet: lower it as the compiler improves.
#[test]
fn json_number_check_is_straight_line() {
    use super::block::BlockOp::*;
    let program = json_program();
    let check = &program.block_programs()[0].checks[0].program;
    let fills = check.ops.iter().filter(|op| matches!(op, Fill(..))).count();
    let other: Vec<&block::BlockOp> = check.ops.iter().filter(|op| !matches!(op, Eq(_) | Class { .. } | Const(_) | Within(0) | Not(_) | And(..) | Or(..) | AndNot(..) | Prev(_, 1) | Fill(..))).collect();
    eprintln!("json number check: {} operations, {fills} fills", check.ops.len());
    assert_eq!((fills, other.len()), (3, 0), "{other:?}");
    assert!(check.ops.len() <= JSON_NUMBER_CHECK_OPS, "{} operations (budget {JSON_NUMBER_CHECK_OPS})", check.ops.len());
}

const JSON_NUMBER_CHECK_OPS: usize = 100;

/// JSON's number check, as straight-line operations, equals the reference's grammar on every
/// word of up to five bytes over `-01.eEx`, and on longer ones: run on the block interpreter over
/// the token's bytes, it fails exactly the words the reference refuses.
#[test]
fn json_number_check_equals_the_reference() {
    let program = json_program();
    let check = &program.block_programs()[0].checks[0].program;
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let tables = std::collections::HashMap::new();
    let alphabet = b"-01.eEx";
    let mut words: Vec<Vec<u8>> = vec![];
    let mut frontier: Vec<Vec<u8>> = vec![vec![]];
    for _ in 0..5 {
        frontier = frontier.iter().flat_map(|w| alphabet.iter().map(move |&b| [w.as_slice(), &[b]].concat())).collect();
        words.extend(frontier.iter().cloned());
    }
    words.extend([&b"-12.5e-30"[..], b"100000000000000000000000000000000000000000000000000000000.5", b"1.e5", b"0.0e0", b"-0e-0", b"1e+5"].map(<[u8]>::to_vec));
    // Tokens over 64 bytes: the check runs word by word, its carries passed on, as tier 1 runs
    // over a file; never sent to tree-sitter (decision 0052). Lengths around the word's edges.
    for n in [63, 64, 65, 127, 128, 129, 200, 1000] {
        let digits = vec![b'7'; n];
        words.push([&b"-1"[..], &digits].concat());
        words.push([&b"1."[..], &digits, b"e-", &digits].concat());
        words.push([&b"0"[..], &digits].concat());
        words.push([&digits[..], b"."].concat());
        words.push([&digits[..], b"e"].concat());
        words.push([&digits[..], b"x", &digits].concat());
        words.push([&digits[..], b"-"].concat());
        words.push([&digits[..n - 1], b"-", &digits].concat());
    }
    let mut refused = 0;
    for w in &words {
        // The word alone is a file of one number token (or a keyword, which no word here is).
        let out = ::bitlex::eval::Ev::run(&stmts, w, &tables).expect("evaluates");
        let want = out.errors.iter().any(|e| e.contains("fails its check"));
        let got = check.interpret(w)[0].iter().any(|&x| x != 0);
        assert_eq!(got, want, "{:?}", String::from_utf8_lossy(w));
        refused += usize::from(want);
    }
    assert!(refused > 0 && refused < words.len());
}

/// A check whose byte machine cycles through two or more states (`('a' 'b')*`) would need a loop
/// over the token's bytes: the compiler refuses it. One whose only cycles are runs (`digit+`,
/// `'-' digit* '.'`) compiles to straight-line operations.
#[test]
fn a_check_that_needs_a_byte_loop_is_refused() {
    use ::bitlex::syntax::Gram;
    let classes = std::collections::HashMap::from([("digit".to_owned(), vec![(0x30, 0x39)])]);
    let lit = |s: &str| Gram::Lit(s.to_owned());
    let compile = |g: Gram| super::grammar::byte_check(&g, &classes).and_then(|(accept, arms)| super::grammar::check_program(&accept, &arms));
    let cycle = compile(Gram::Star(Box::new(Gram::Seq(vec![lit("a"), lit("b")])))).expect_err("a two-state cycle");
    assert!(cycle.contains("loop over the bytes"), "{cycle}");
    for g in [Gram::Plus(Box::new(Gram::Sym("digit".into()))), Gram::Seq(vec![lit("-"), Gram::Star(Box::new(Gram::Sym("digit".into()))), lit(".")])] {
        let p = compile(g).expect("runs only");
        assert!(p.ops.iter().all(|op| !matches!(op, block::BlockOp::Toggle(_) | block::BlockOp::Escaped(_))), "{:?}", p.ops);
    }
}
