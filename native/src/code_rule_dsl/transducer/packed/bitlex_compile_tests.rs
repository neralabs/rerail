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
            assert_eq!(cut(got[2].clone(), bytes.len()), want, "{executor}, {} bytes", bytes.len());
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
    let carried = compiled.rows.lines().filter(|l| [" = fill ", " = toggle ", " = after_odd_run ", " = prev "].iter().any(|op| l.contains(op))).count();
    eprintln!("{ops} operations, {carried} carried or prev, {} outputs", compiled.rows.lines().filter(|l| l.contains(" output ")).count());
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let prepared = Executors::new(&p);
    let mut cases = inputs();
    cases.extend(JSON_CASES.iter().map(|c| c.to_vec()));
    let (mut compared, mut rejected, mut conservative) = (0, 0, 0);
    let mut skipped: Vec<String> = Vec::new();
    for bytes in cases {
        let n = bytes.len();
        let tables = std::collections::HashMap::new();
        let out = ::bitlex::eval::Ev::run(&stmts, &bytes, &tables).unwrap_or_else(|e| panic!("evaluates: {e}"));
        let tree = out.tree.as_ref().expect("a tree section");
        let token_mask = |kind: &str, last: bool| {
            let mut w = vec![0u64; n.div_ceil(64)];
            for t in out.tokens.iter().filter(|t| t.kind == kind) {
                let i = if last { t.end - 1 } else { t.start };
                w[i / 64] |= 1 << (i % 64);
            }
            w
        };
        let want = |label: &str| -> Option<Vec<u64>> {
            let words: Vec<&str> = label.split(' ').collect();
            match words.as_slice() {
                ["node", k, side] => Some(match tree.nodes.iter().find(|r| r.0 == *k) {
                    Some(r) => if *side == "start" { r.1.w.clone() } else { r.2.w.clone() },
                    None => token_mask(k, *side == "end"),
                }),
                ["code", _, name] => Some(tree.facts.iter().find(|f| f.1 == *name).expect("a fact").3.w.clone()),
                _ => None,
            }
        };
        let (runs, over) = prepared.run(&bytes, true);
        skipped.extend(over);
        for (executor, got) in runs {
            let falls = cut(got[0].clone(), n).iter().any(|&w| w != 0);
            if !out.errors.is_empty() {
                assert!(falls, "{executor}, {n} bytes: the evaluator rejects ({:?}) but the file does not fall back", out.errors);
                rejected += 1;
                continue;
            }
            if falls {
                conservative += 1;
                continue;
            }
            for (label, k) in &compiled.labels {
                if let Some(want) = want(label) {
                    assert_eq!(cut(got[usize::from(*k)].clone(), n), cut(want, n), "{label}, {executor}, {n} bytes: {:?}", String::from_utf8_lossy(&bytes));
                }
            }
            compared += 1;
        }
    }
    skipped.sort();
    skipped.dedup();
    eprintln!("{compared} compared, {rejected} rejected, {conservative} falling back where the evaluator does not; over the launch budget: {skipped:?}");
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
            assert_eq!(msl.matches("kernel void tempo_").count(), 1);
            let lines = msl.lines().count();
            assert!(lines < 12 * native.insts.len(), "{}-byte {strategy:?}: {lines} lines for {} instructions", width.bytes(), native.insts.len());
        }
    }
}
