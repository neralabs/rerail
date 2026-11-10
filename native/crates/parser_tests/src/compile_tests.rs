//! The Bitlex compiler (`bitlex_compile`), checked against the reference evaluator
//! (`tools/bitlex`): each named mask of the compiled block program equals the evaluator's mask on
//! every input and every executor (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).

use crate::block_tests::{Executors, executors, inputs, runs};
#[allow(unused_imports)]
use rerail_compiler::*;
use rerail_compiler::program::Words as Program;
use rerail_compiler::rir_asm as rir;
use rerail_compiler::bitlex_masks as bitlex_compile;
#[cfg(target_os = "macos")]
use rerail_metal as metal_program_backend;

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

/// Numbers (`number`): planes of a place, carried back and forth, subtracted and compared, in a
/// fixed point too; each mask equals the evaluator's, which holds the numbers whole.
const NUMBERS: &str = r#"
lexer numbers
class tick = '`'
mask last  = tick & ~next(tick)
number end = back_fill(last, ~tick, position)
number rem = end - position
mask long  = tick & at_least(rem, 3)
number n   = fill(long, long, rem)
mask close = tick & at_least(rem, n) & ~long
mask over  = above(n, rem)
mask far   = at_least(position, 1000) & ~at_least(position, 66000)
mask odd   = above(end, 65537)
rounds until settled {
  carry h  = empty
  mask o   = long & ~h
  number m = fill(o, o, rem)
  mask c   = tick & at_least(rem, m) & ~o
  mask f   = fill(o, c)
  next h   = prev(f)
}
"#;

#[test]
fn numbers_equal_the_evaluator_on_every_executor() {
    let names = ["long", "close", "over", "far", "odd", "o", "c", "f"];
    let stmts = ::bitlex::syntax::parse(NUMBERS).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names, None).unwrap_or_else(|e| panic!("compiles: {e}"));
    assert!(compiled.rows.contains("position 6") && !compiled.rows.contains("position 5"), "planes from bit 6 are the block's place\n{}", compiled.rows);
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let mut cases = inputs();
    let fences = |n: usize| {
        let mut v = Vec::new();
        for k in 0..n {
            v.extend(std::iter::repeat_n(b'`', 1 + k * 7 % 9));
            v.extend(std::iter::repeat_n(b'x', k * 13 % 70));
        }
        v
    };
    cases.extend([b"a````b``c`````d````e`````".to_vec(), b"````````".to_vec(), fences(60), [vec![b'.'; 65_530], fences(40)].concat()]);
    for bytes in cases {
        let want = evaluated(&stmts, &bytes, &names);
        for (executor, got) in Executors::new(&p).run(&bytes, false).0 {
            for (k, name) in names.iter().enumerate() {
                assert_eq!(cut(got[k].clone(), bytes.len()), want[k], "{name}, {executor}, {} bytes", bytes.len());
            }
        }
    }
}

/// Values made from the position alone, read after a backward pass (stored by the first pass and
/// loaded): `aligned(_, 3)`'s phases and `end - position`'s previous position, on every executor,
/// Metal's forward carry and backward passes included.
#[test]
fn positions_after_a_backward_pass_equal_the_evaluator_on_every_executor() {
    let aligned = "lexer t\nclass d = '0'..'9'\nclass q = '\"'\nclass sp = ' '\nmask s = back_fill(q, sp)\nmask a = aligned(d, 3)\nmask b = aligned(s, 3)\n";
    for (text, names) in [(aligned, &["a", "b"][..]), (NUMBERS, &["long", "close", "over", "far", "odd"][..])] {
        let stmts = ::bitlex::syntax::parse(text).expect("parses");
        let compiled = bitlex_compile::compile(&stmts, names, None).unwrap_or_else(|e| panic!("compiles: {e}"));
        let rows = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
        let p = rir::assemble(&rows).unwrap_or_else(|e| panic!("{e}\n{rows}"));
        let mut cases: Vec<Vec<u8>> = vec![b"12\"3 4567\" 8 9\"0".to_vec(), b"a````b``c`````d````e`````".to_vec()];
        cases.extend(runs());
        cases.push([vec![b'1'; 70_000], b"`````\"x\" ```".to_vec()].concat());
        for bytes in cases {
            let want = evaluated(&stmts, &bytes, names);
            // Metal runs a backward pass in its forward carry only (`run_passes`), not in its
            // interpreter or generated kernels.
            #[allow(unused_mut)]
            let mut got = Executors::new(&p).run(&bytes, false).0;
            #[cfg(target_os = "macos")]
            {
                use metal_program_backend::block_program::serial::{Segments, run_serial_split};
                let native = native_ir::lower(&p, 0, 0).expect("lowers").block_programs.remove(0);
                let file = [block::BatchFile { bytes: &bytes, program: 0 }];
                for blocks in [3, 29] {
                    let run = run_serial_split(&[&native], &file, Segments { blocks, warm: 1 }).unwrap_or_else(|e| panic!("the forward carry runs: {e:?}"));
                    got.push((format!("metal forward carry, segments of {blocks}"), run.run.files[0].outputs.clone()));
                }
            }
            for (executor, got) in got {
                for (k, name) in names.iter().enumerate() {
                    assert_eq!(cut(got[k].clone(), bytes.len()), want[k], "{name}, {executor}, {} bytes", bytes.len());
                }
            }
        }
    }
}

/// The whole of `json.bitlex`: on every input and every executor, a file the evaluator rejects
/// falls back, and for one that neither rejects, each row's masks equal the evaluator's: the
/// tree section's starts, ends and facts, and each token kind's starts and last bytes.
const JSON: &str = include_str!("../../../../docs/parser-generators/lexers/json.bitlex");

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

/// Tier 1's stream, on every executor, is the evaluator's tokens (decision 0087): the starts
/// extract is every token's first byte; the lasts extract is the last byte of each token whose
/// end tier 2 reads from it (a row whose tokens are not one byte and not all keywords).
#[test]
fn json_stream_equals_the_evaluator_on_every_executor() {
    use ::bitlex::syntax::Stmt;
    let (p, compiled) = json_compiled();
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let prepared = Executors::new(&p);
    let label = |name: &str| usize::from(compiled.labels.iter().find(|l| l.0 == name).unwrap_or_else(|| panic!("no {name}")).1);
    let (starts, lasts) = (label(bitlex_compile::STREAM_START), label(bitlex_compile::STREAM_LAST));
    // The kinds whose tokens' last bytes are in the stream: rows with a last byte of their own,
    // and their keywords. A row of keywords only is one too: its token runs to its own last byte,
    // which tier 2 compares with the word (`truee` is not `true`).
    let mut late: Vec<String> = Vec::new();
    for s in &stmts {
        if let Stmt::Token { kind, last, through, .. } = s
            && (last.is_some() || through.is_some())
        {
            late.push(kind.clone());
            if let Some(words) = stmts.iter().find_map(|k| match k {
                Stmt::Keywords { kind: k, words, .. } if k == kind => Some(words),
                _ => None,
            }) {
                late.extend(words.iter().map(|(w, alias)| alias.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned())));
            }
            late.dedup();
        }
    }
    assert_eq!(late, ["string_content", "escape_sequence", "number", "true", "false", "null"]);
    let mut cases = inputs();
    cases.extend(JSON_CASES.iter().map(|c| c.to_vec()));
    let (mut compared, mut rejected, mut conservative) = (0, 0, 0);
    for bytes in cases {
        let n = bytes.len();
        let out = ::bitlex::eval::Ev::run(&stmts, &bytes, &std::collections::HashMap::new()).unwrap_or_else(|e| panic!("evaluates: {e}"));
        if !out.errors.is_empty() {
            rejected += 1;
            continue;
        }
        let mut want_starts = vec![0u64; n.div_ceil(64)];
        let mut want_lasts = vec![0u64; n.div_ceil(64)];
        for t in &out.tokens {
            want_starts[t.start / 64] |= 1 << (t.start % 64);
            if late.contains(&t.kind) {
                want_lasts[(t.end - 1) / 64] |= 1 << ((t.end - 1) % 64);
            }
        }
        let (runs, over) = prepared.run(&bytes, true);
        assert!(over.is_empty(), "over the launch budget: {over:?}");
        for (executor, got) in runs {
            if compiled.fallbacks.iter().any(|&k| cut(got[usize::from(k)].clone(), n).iter().any(|&w| w != 0)) {
                conservative += 1;
                continue;
            }
            let at = format!("{executor}, {n} bytes: {:?}", String::from_utf8_lossy(&bytes));
            assert_eq!(cut(got[starts].clone(), n), want_starts, "starts, {at}");
            assert_eq!(cut(got[lasts].clone(), n), want_lasts, "lasts, {at}");
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
        use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_with};
    let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).expect("compiles");
    let text = compiled.text();
    let p = rir::assemble(&text).expect("assembles");
    let native = native_ir::lower(&p, 0, 0).expect("lowers").block_programs.remove(0);
    for width in LaneWidth::ALL {
        for strategy in ClassStrategy::ALL {
            let msl = generate_with(&native, width, strategy);
            assert_eq!(msl.matches("kernel void ").count(), 1);
            let lines = msl.lines().count();
            // A fixed preamble (the kernel's arguments, tables and stores), then a few lines per
            // instruction in each of three bodies (general, full group, ASCII full group).
            assert!(lines < 200 + 15 * native.insts.len(), "{}-byte {strategy:?}: {lines} lines for {} instructions", width.bytes(), native.insts.len());
        }
    }
}

/// A batch (decision 0066): files of several languages, interleaved, each under its own program,
/// in one launch. Every file's outputs and positions equal its program's CPU interpreter on that
/// file alone, on the Metal interpreter and on generated MSL at every lane width. With the
/// default polls it is one launch; with one poll per predecessor and none after it, groups leave unfinished and the launch is repeated; the
/// results are the same.
#[test]
#[cfg(target_os = "macos")]
fn a_batch_of_files_and_programs_is_one_launch() {
    use metal_program_backend::block_program::{ClassStrategy, Executor, LaneWidth, SPINS, run_batch};
    let programs = batch_programs();
    let blocks: Vec<&block::BlockProgram> = programs.iter().map(|p| &p.block_programs[0]).collect();
    let natives: Vec<block_native::BlockNative> = programs.iter().map(|p| native_ir::lower(p, 0, 0).expect("lowers").block_programs.remove(0)).collect();
    let natives: Vec<&block_native::BlockNative> = natives.iter().collect();
    let bytes = batch_bytes();
    let files = batch_files(&bytes, blocks.len());
    let want: Vec<_> = files.iter().map(|f| blocks[f.program].run(f.bytes)).collect();
    for width in LaneWidth::ALL {
        for executor in [Executor::Interpreter, Executor::Generated(ClassStrategy::Lookup), Executor::Generated(ClassStrategy::Planes)] {
            for spins in [SPINS, 0] {
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
    use crate::block_tests::STAGE1;
        let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let compiled = |src: &str, names: &[&str], symbols: Option<&bitlex_compile::Symbols<'_>>| {
        let stmts = ::bitlex::syntax::parse(src).expect("parses");
        let text = bitlex_compile::compile(&stmts, names, symbols).unwrap_or_else(|e| panic!("compiles: {e}")).text();
        rir::assemble(&text).expect("assembles")
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
    bytes.extend(crate::block_tests::runs());
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
    let blocks: Vec<&block::BlockProgram> = programs.iter().map(|p| &p.block_programs[0]).collect();
    let compiled: Vec<_> = programs
        .iter()
        .map(|p| native_cpu_block::compile(&native_ir::lower(p, 0, 0).expect("lowers").block_programs.remove(0)).expect("compiles"))
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
            if name == "interpreter" {
                // Cranelift has no lowering of `extract` rows (docs/parser-generators/20-the-ir.md).
                assert_eq!(got.positions, want.positions, "{what}");
            }
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
    let native = native_ir::lower(&program, 0, 0).expect("lowers").block_programs.remove(0);
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

/// JSON's tier 2 tells every token's row by its first byte: no `ROW` word. Its rows share bytes
/// (a string's text and a number begin with nearly any), but never in one state: a string's
/// text is read inside a string, a number outside (the toggle's modes, `row_modes`), and a
/// number never begins with a quote (an escaped one follows a word's `\\`, `bitlex_prove`).
#[test]
fn json_tier2_switches_on_the_first_byte_alone() {
        let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let text = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).unwrap_or_else(|e| panic!("compiles: {e}")).text();
    let rows: Vec<&str> = text.lines().filter(|l| l.split_whitespace().nth(1) == Some("ROW")).collect();
    assert!(text.contains(" SWITCH "), "no tier 2 words:\n{text}");
    assert!(rows.is_empty(), "{} ROW words: {rows:?}", rows.len());
}

/// JSON's fallback outputs (`bitlex_compile::Compiled::fallbacks`).
fn json_fallbacks() -> Vec<u16> {
        let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).expect("compiles").fallbacks
}


// --- The shape of a compiled language (decision 0069) -----------------------------------------
//
// A Bitlex file compiles to simdjson's shape: straight-line bit operations, a carry only where the
// language needs one. These rules hold for every compiled program; JSON's budgets are ratchets,
// lowered as the compiler improves, never raised.

/// A block program's violations of the shape rules, each a short line.
fn shape_violations(p: &block::BlockProgram) -> Vec<String> {
    use rerail_compiler::block::BlockOp::*;
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
    let p = &program.block_programs[0];
    let violations = shape_violations(p);
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
    let prevs = p.ops.iter().filter(|op| matches!(op, block::BlockOp::Prev(..))).count();
    let (ops, words) = (p.ops.len(), crate::block_tests::carried_words(p));
    eprintln!("json: {ops} operations, {prevs} prevs, {words} carried words, {} outputs", p.outputs.len());
    assert!(ops <= JSON_OPS && words <= JSON_CARRIED_WORDS, "json: {ops} operations (budget {JSON_OPS}), {words} carried words (budget {JSON_CARRIED_WORDS})");
}

/// JSON's carried state is bits (decision 0070): one per carried operation, and `k` per value a
/// `prev` looks back on. The information that crosses a block boundary is about 18 bits (4 of
/// lexical state: where in a string, number or literal, or between tokens; 14 of nesting: a
/// depth to 13 and whether each open bracket is '{'), plus the bits the rules look back on. A
/// ratchet, lowered as the compiler stops restating state.
#[test]
fn json_state_is_bits() {
    let program = json_program();
    let bits = crate::block_tests::state_bits(&program.block_programs[0]);
    eprintln!("json: {bits} state bits (budget {JSON_STATE_BITS})");
    assert!(bits <= JSON_STATE_BITS, "json: {bits} state bits (budget {JSON_STATE_BITS})");
}

/// JSON's state budget (a ratchet): 104 bits in one tier; 4 in two (decision 0071); 5 since a
/// keyword-only row's tokens end at their own last bytes (an escape's last byte, one byte after
/// its backslash, is carried into the next block's last-bytes stream); 15 since the number check
/// is tier 1's masks (doc 18 §4): a bit per state it looks back on and per run it fills.
const JSON_STATE_BITS: usize = 15;

/// JSON's budgets (ratchets: lower them as the compiler improves). 37 operations rose to 40 when
/// tier 1 began to hand tier 2 its stream (decision 0087): the starts' four ORs, as simdjson's
/// structural mask; one OR of the last bytes; and a guard of two (a number starting at a quote)
/// that a sharper first-byte analysis would drop. The four token-row outputs went. 42 and 5
/// carried words when escapes' last bytes joined the stream (a `prev` by one and an OR): a row of
/// keywords only is checked over its token's whole length, so `truee` is not `true`. A fixed
/// length the compiler can prove would drop them. 85 and 15 when the number check moved from a
/// program of its own, run per number in tier 2, into tier 1's masks (`JSON_NUMBER_CHECK_OPS`).
/// 86 and 16 when the check's span fills from the starts it takes (`start & taken`), one `and`
/// and its field: the exemption of other rows' starts went with it.
const JSON_OPS: usize = 86;
const JSON_CARRIED_WORDS: usize = 16;

#[test]
#[ignore = "a dump; run by name"]
fn dump_json_rows() {
    std::fs::write(std::env::var("RERAIL_DUMP").expect("RERAIL_DUMP"), rir::disassemble(&json_program())).expect("writes");
}

/// JSON's Cranelift loop: no vector `mvn`, one carried word per prev base and per carry, and its
/// stack loads and stores within budget (a ratchet, to reach 0 as the carried words fit).
#[test]
#[cfg(target_arch = "aarch64")]
fn json_loop_keeps_values_in_registers() {
    let program = json_program();
    let native = native_ir::lower(&program, 0, 0).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = crate::block_tests::machine_loop(&compiled.loops_disassembly());
    let (stack, mvn) = crate::block_tests::loop_costs(&lines);
    eprintln!("json loop: {} instructions, {stack} stack loads or stores, {mvn} vector mvn", lines.len());
    assert_eq!(mvn, 0, "json's loop inverts vectors");
    // The state is packed: one carried word per 64 state bits (decision 0070).
    let bits = crate::block_tests::state_bits(&program.block_programs[0]);
    assert_eq!(crate::block_tests::carried_params(&compiled.clif) - crate::block_tests::sink_params(&native), bits.div_ceil(64), "json's carried state is not packed");
    assert!(lines.len() <= JSON_LOOP && stack <= JSON_LOOP_STACK, "json loop: {} instructions (budget {JSON_LOOP}), {stack} stack loads or stores (budget {JSON_LOOP_STACK})", lines.len());
}

/// JSON's loop budgets (ratchets): lower them as the program and the lowering improve. Packing
/// the carried state (decision 0070) raised the instructions once, 1915 -> 1939, for 405 -> 164
/// stack operations, at an equal or lower CPU time in every paired timing. Two tiers (decision
/// 0049) lowered them to 483 and 6, one structural class and kinds in tier 2 to 253 and 0, and a
/// full-block body with folded logic to 178, and one word token with the number check in tier 2
/// (decision 0086) to 174; the stream's two extracts (decision 0087) to 182, three outputs
/// in place of ten, and two counts. The target is stage 1's 150.
///
/// Those counts were the loop's tail only: the finder took the largest block that branched to
/// itself, the sinks after the ASCII branch's merge. Measured over the whole body (both
/// bodies of the ASCII branch, the flattenings once), the code before the gated regions'
/// calls has 564 instructions and 4 stack loads or stores; with the delayed outputs not held
/// and the regions called where it is free, 566 and 4.
const JSON_LOOP: usize = 566;
const JSON_LOOP_STACK: usize = 4;

// --- Tier 1 is simdjson's stage 1 (decision 0071) ---------------------------------------------
//
// A language's block program (tier 1) does only the work that needs every byte: one
// classification, the string state, the token starts and the per-byte errors. Everything about
// a token (its kind, its text's checks, its place in the tree) is tier 2's, once per token.

/// A tier-1 program's violations: more string state than simdjson's (one `escaped`, one
/// `toggle`), a `fill` (a run is a token's; tier 2 reads it from the token's text), or a look-back
/// past the byte before (a token start looks one byte back, as simdjson's `follows`).
fn tier1_violations(p: &block::BlockProgram) -> Vec<String> {
    use rerail_compiler::block::BlockOp::*;
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
#[ignore = "red guard (decision 0071): on at step 3, when json.bitlex's dense part is stage 1's"]
fn json_tier1_is_simdjsons_stage1() {
    let program = json_program();
    let p = &program.block_programs[0];
    let violations = tier1_violations(p);
    let bits = crate::block_tests::state_bits(p);
    let native = native_ir::lower(&program, 0, 0).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = crate::block_tests::machine_loop(&compiled.loops_disassembly());
    let (stack, _) = crate::block_tests::loop_costs(&lines);
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
    let got = json_compiled().1.machine;
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

/// Tier 2's states and arms are the compiler's (doc 18, decisions 0085 and 0053): `grammar` makes
/// them and `bitlex_tier2` lowers them to the transducer's words, which tier 2 runs as. Nothing
/// else reads them, and nothing walks them to parse a file. A file outside this list that reads
/// them fails here.
#[test]
fn only_the_compiler_reads_tier2s_states() {
    // The compiler crate's `grammar` and `bitlex_tier2`, its Bitlex compiler (`bitlex_masks`, which
    // hands the table to them), and this file, which names them.
    const ALLOWED: [&str; 4] = ["grammar.rs", "bitlex_masks.rs", "bitlex_tier2.rs", "compile_tests.rs"];
    // The engine's transducer and every crate beside it.
    let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // The table's types and its arms (`.states` is a common name; a state row is read through
    // its type).
    let needles = [".arms", "ArmRow", "StateRow", "grammar::Act", "Act::Open", "Act::Close", "Act::Leaf"];
    let mut readers = Vec::new();
    let mut dirs = vec![native.join("src/code_rule_dsl/transducer"), native.join("crates")];
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

/// JSON with its `check` line replaced by `with` (or dropped), compiled whole.
fn json_with_check(with: Option<&str>) -> Result<(Program, bitlex_compile::Compiled), String> {
    let text: String = JSON
        .lines()
        .map(|l| if l.starts_with("check number") { with.unwrap_or("") } else { l })
        .collect::<Vec<_>>()
        .join("\n");
    json_compiled_text(&text)
}

/// JSON's number check (doc 18 §4) is tier 1's: masks computed with the rest of the block
/// program, a state's mask per state of its byte machine, joining the fallback. One `fill` per
/// run (the integer, the fraction, the exponent) and one for the tokens it leaves to tier 2's
/// keywords; `prev`s by one byte, and logic; no other carry. Tier 2 has no word for it. Its
/// size is a ratchet: lower it as the compiler improves.
#[test]
fn json_number_check_is_tier1_masks() {
    use rerail_compiler::block::BlockOp::*;
    let (with, _) = json_compiled();
    let (without, _) = json_with_check(None).expect("compiles without the check");
    let (a, b) = (&with.block_programs[0], &without.block_programs[0]);
    let fills = |p: &block::BlockProgram| p.ops.iter().filter(|op| matches!(op, Fill(..))).count();
    let added: Vec<&block::BlockOp> = a.ops.iter().filter(|op| !b.ops.contains(op)).collect();
    let other: Vec<&&block::BlockOp> = added.iter().filter(|op| !matches!(op, Eq(_) | Class { .. } | Const(_) | Within(_) | Not(_) | And(..) | Or(..) | AndNot(..) | Prev(_, 1) | Fill(..))).collect();
    eprintln!("json number check: {} operations, {} fills", a.ops.len() - b.ops.len(), fills(a) - fills(b));
    assert_eq!((fills(a) - fills(b), other.len()), (4, 0), "{other:?}");
    assert!(a.ops.len() - b.ops.len() <= JSON_NUMBER_CHECK_OPS, "{} operations (budget {JSON_NUMBER_CHECK_OPS})", a.ops.len() - b.ops.len());
    let text = rir::disassemble(&with);
    assert!(!text.lines().any(|l| l.split_whitespace().nth(1) == Some("CHECK")), "tier 2 checks a token");
}

/// The check's operations (a ratchet). 100 as a program of its own run per number in tier 2;
/// 43 as masks in tier 1, its arms into a state on the same bytes sharing one `prev`; 44 when its
/// span fills from the starts it takes.
const JSON_NUMBER_CHECK_OPS: usize = 44;

/// JSON's number check, as tier 1's masks, equals the reference's grammar on every word of up
/// to five bytes over `-01.eEx`, and on longer ones: a file of the word alone is parsed, both
/// tiers on their interpreters, exactly when the reference parses it, to the same nodes.
#[test]
fn json_number_check_equals_the_reference() {
    let (p, _) = json_compiled();
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let alphabet = b"-01.eEx";
    let mut words: Vec<Vec<u8>> = vec![];
    let mut frontier: Vec<Vec<u8>> = vec![vec![]];
    for _ in 0..5 {
        frontier = frontier.iter().flat_map(|w| alphabet.iter().map(move |&b| [w.as_slice(), &[b]].concat())).collect();
        words.extend(frontier.iter().cloned());
    }
    words.extend([&b"-12.5e-30"[..], b"100000000000000000000000000000000000000000000000000000000.5", b"1.e5", b"0.0e0", b"-0e-0", b"1e+5", b"[1,-2,3e4]", b"[1 2]", b"[truee]", b"[nul]", b"[true,1x]"].map(<[u8]>::to_vec));
    // Tokens over 64 bytes, and at the blocks' edges: the masks carry from block to block.
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
        words.push([&b"["[..], &vec![b' '; n - 2], b"1,2.5]"].concat());
    }
    let mut refused = 0;
    for w in &words {
        let want = reference_nodes(&stmts, w);
        assert_eq!(tier2_nodes(&p, w), want, "{:?}", String::from_utf8_lossy(w));
        refused += usize::from(want.is_none());
    }
    assert!(refused > 0 && refused < words.len());
}

/// A check whose byte machine cycles through two or more states (`('a' 'b')*`) would need a loop
/// over the token's bytes: the compiler refuses it. JSON's, whose only cycles are runs, compiles.
#[test]
fn a_check_that_needs_a_byte_loop_is_refused() {
    let cycle = json_with_check(Some("check number = ('-' '0')+")).map(|_| ()).expect_err("a two-state cycle");
    assert!(cycle.contains("a cycle through two or more states"), "{cycle}");
    json_with_check(Some("check number = digit+")).expect("runs only");
}

// --- Tier 2: the transducer's words (decision 0087) ----------------------------------------------

/// JSON compiled whole: its program, and the compiler's output (labels, fallbacks, machine).
fn json_compiled() -> (Program, bitlex_compile::Compiled) {
    json_compiled_text(JSON).unwrap_or_else(|e| panic!("compiles: {e}"))
}

/// A Bitlex text with JSON's tree compiled whole.
fn json_compiled_text(json: &str) -> Result<(Program, bitlex_compile::Compiled), String> {
        let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => Some(grammar.id_for_node_kind(name, false)).filter(|&id| id != 0),
        id => Some(id),
    };
    let field = |name: &str| grammar.field_id_for_name(name).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let stmts = ::bitlex::syntax::parse(json).map_err(|e| format!("{e:?}"))?;
    let compiled = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols))?;
    let text = compiled.text();
    Ok((rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}")), compiled))
}

/// A node as ids: kind, field, supertypes (sorted), start, end, size.
type IdNode = (u16, u16, Vec<u16>, usize, usize, usize);

/// The reference parser's nodes for `bytes` as ids, or `None` where the file goes to tree-sitter.
fn reference_nodes(stmts: &[::bitlex::syntax::Stmt], bytes: &[u8]) -> Option<Vec<IdNode>> {
        let grammar = tree_sitter::Language::new(tree_sitter_json::LANGUAGE);
    let kind = |name: &str| match grammar.id_for_node_kind(name, true) {
        0 => grammar.id_for_node_kind(name, false),
        id => id,
    };
    let out = ::bitlex::eval::Ev::run(stmts, bytes, &std::collections::HashMap::new()).ok()?;
    if !out.errors.is_empty() {
        return None;
    }
    let nodes = ::bitlex::tree::nodes(out.tree.as_ref()?, &out.tokens, bytes.len()).ok()?;
    Some(
        nodes
            .iter()
            .map(|n| {
                let field = n.field.as_deref().map_or(0, |f| grammar.field_id_for_name(f).expect("a field").get());
                let mut supers: Vec<u16> = n.supertypes.iter().map(|s| kind(s)).collect();
                supers.sort_unstable();
                (kind(&n.kind), field, supers, n.start, n.end, n.size)
            })
            .collect(),
    )
}

/// JSON files for tier 2: the cases, small files of every kind and error, nesting, and tokens
/// longer than a 64-byte block.
fn json_tier2_files() -> Vec<Vec<u8>> {
    let mut files: Vec<Vec<u8>> = JSON_CASES.iter().map(|c| c.to_vec()).collect();
    let long = "9".repeat(150);
    files.extend(
        [
            "[]", "{}", "[[]]", "[{}]", "{\"a\":[]}", " 1 ", "true", "null", "[true,false,null]", "\"\"", "\"\\n\"", "\"a\\\"b\"",
            "\"\\x\"", "[1,]", "{\"a\"}", "{\"a\":}", "[1 2]", "-", "1.", "1.e5", "-0.0e-0", "01", "tru", "truex", "[nul]",
            "{\"a\":1,\"b\":{\"c\":[1,{\"d\":null}]}}", "  [ 1 , 2 ]  \n", "[\"a\" , \"b\"]",
        ]
        .map(|s| s.as_bytes().to_vec()),
    );
    files.push(format!("[{long}, -{long}.{long}e-{long}]").into_bytes());
    files.push(format!("{{\"{long}\": \"{long}\\t{long}\"}}").into_bytes());
    files
}

/// Tier 1 on the interpreter, then tier 2's words on theirs: the nodes, or `None` where the file
/// goes to tree-sitter (a fallback bit, or a word that refuses).
fn tier2_nodes(p: &Program, bytes: &[u8]) -> Option<Vec<IdNode>> {
    let block = &p.block_programs[0];
    let run = block.run_files(&[bytes]).remove(0);
    let nodes = stream::run(p, bytes, &run).ok()?;
    Some(
        nodes
            .into_iter()
            .map(|n| {
                let mut supers = n.supers;
                supers.sort_unstable();
                (n.kind, n.field, supers, n.start as usize, n.end as usize, n.size as usize)
            })
            .collect(),
    )
}

/// JSON's tier 2 is the transducer's words (decision 0087): it reads the stream, switches on
/// bytes and writes nodes; it has no word of the tree walker's, and its loops all pass a `TOKEN`
/// (the verifier's rule, which the program passed to assemble).
#[test]
fn json_tier2_is_transducer_words() {
    let (p, _) = json_compiled();
    let text = rir::disassemble(&p);
    let again = rir::assemble(&text).expect("reassembles");
    assert_eq!((&again.words, &again.block_programs, &again.strings), (&p.words, &p.block_programs, &p.strings), "the words round trip");
    let words: Vec<&str> = text.lines().filter(|l| l.trim_start().starts_with(|c: char| c.is_ascii_digit())).collect();
    for want in ["TOKEN", "SWITCH", "OPEN", "LEAF", "CLOSE", "PUSH", "RETURN", "ACCEPT", "WORD"] {
        assert!(words.iter().any(|l| l.split_whitespace().nth(1) == Some(want)), "no {want}:\n{}", words.join("\n"));
    }
    for banned in ["FORK", "CHILD", "NEXT", "DESCEND", "CAPTURE", "EMIT", "CHECK"] {
        assert!(!words.iter().any(|l| l.split_whitespace().nth(1) == Some(banned)), "{banned}:\n{}", words.join("\n"));
    }
}

/// JSON's tier 2, on the words' interpreter, writes the reference parser's nodes, and goes to
/// tree-sitter where it does: the test cases, nesting, every token kind, long numbers.
#[test]
fn json_tier2_nodes_equal_the_reference() {
    let (p, _) = json_compiled();
    let stmts = ::bitlex::syntax::parse(JSON).expect("parses");
    let files = json_tier2_files();
    let mut parsed = 0;
    for f in &files {
        let want = reference_nodes(&stmts, f);
        let got = tier2_nodes(&p, f);
        assert_eq!(got, want, "{}", String::from_utf8_lossy(f));
        parsed += usize::from(want.is_some());
    }
    assert!(parsed > files.len() / 2, "{parsed} of {} parsed", files.len());
}

/// JSON on the compiled path: tier 1 and tier 2 both Cranelift (the number check inlined in
/// tier 2), each run equal to the interpreters'.
#[test]
fn json_compiled_path_equals_the_interpreters() {
    let (p, _) = json_compiled();
    let native = native_ir::lower(&p, 0, 0).expect("lowers");
    let tier1 = native_cpu_block::compile(&native.block_programs[0]).expect("tier 1 compiles");
    let tier2 = native_cpu_stream::compile(&native).expect("tier 2 compiles");
    // JSON has no wraps and no chained returns: it pays for neither (no wrap columns, no
    // reorder). Its loop heads are the stream's and the masks'. The table's final pass
    // (parents and extras) is a function of its own, called once: its loops are not the
    // token loop's.
    assert!(!tier2.wraps);
    assert!(tier2.heads.iter().all(|h| matches!(h.1, native_cpu_shape::Step::Token | native_cpu_shape::Step::Block)), "{:?}", tier2.heads);
    assert_eq!(tier2.own_calls, 1, "the final pass's call alone");
    let files = json_tier2_files();
    for f in &files {
        let want1 = p.block_programs[0].run_files(&[f]).remove(0);
        let got1 = tier1.extract(f);
        assert_eq!(got1, want1, "tier 1: {}", String::from_utf8_lossy(f));
        let want = stream::run(&p, f, &want1).map_err(|r| r.pc);
        let got = tier2.run(f, &got1).map_err(|r| r.pc);
        assert_eq!(got, want, "tier 2: {}", String::from_utf8_lossy(f));
    }
}

/// `rounds until settled`: strings with escapes and `//` comments, each hiding the other's
/// openers, through one carry (the reference's own test lexer). Its masks equal the
/// evaluator's on the CPU executors (Metal refuses a fixed point so far), over every short
/// string of the bytes that matter and long ones that cross blocks.
const ROUNDS: &str = r#"
lexer t
class quote = '"'
class backslash = '\\'
class slash = '/'
class lf = '\n'
mask q0 = quote & ~escaped(backslash)
rounds until settled {
  carry hidden = empty
  mask q = q0 & ~hidden
  mask s = toggle(q)
  mask open = slash & prev(slash & ~s)
  mask line = fill(open, lf)
  next hidden = line
}
mask comment = line | (slash & next(open))
"#;

#[test]
fn rounds_equal_the_evaluator() {
    let names = ["q", "s", "open", "line", "comment"];
    let stmts = ::bitlex::syntax::parse(ROUNDS).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names, None).unwrap_or_else(|e| panic!("compiles: {e}"));
    assert!(compiled.rows.contains(" rounds %"), "a group\n{}", compiled.rows);
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let run = Executors::new(&p);
    let alphabet = b"\"\\/\na";
    let mut cases = inputs();
    for len in 1..=5u32 {
        for code in 0..5usize.pow(len) {
            cases.push((0..len).map(|k| alphabet[code / 5usize.pow(k) % 5]).collect());
        }
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..200 {
        let len = (seed % 300) as usize;
        cases.push(
            (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    alphabet[(seed % 5) as usize]
                })
                .collect(),
        );
    }
    for bytes in cases {
        let want = evaluated(&stmts, &bytes, &names);
        for (executor, got) in run.run(&bytes, false).0 {
            for (k, name) in names.iter().enumerate() {
                assert_eq!(cut(got[k].clone(), bytes.len()), want[k], "{name}, {executor}, {:?}\n{}", String::from_utf8_lossy(&bytes), compiled.rows);
            }
        }
    }
}
