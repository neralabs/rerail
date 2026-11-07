//! The Bitlex compiler (`bitlex_compile`), checked against the reference evaluator
//! (`tools/bitlex`): each named mask of the compiled block program equals the evaluator's mask on
//! every input and every executor (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).

use super::block_tests::{executors, inputs};
use super::*;

/// The masks the compiler handles so far: classes, logic, `prev`, `starts`, `after`, `span`,
/// `toggle` and `after_odd_run`.
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
    names.push("mixed");
    let stmts = ::bitlex::syntax::parse(FORWARD).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names).unwrap_or_else(|e| panic!("compiles: {e}"));
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
    let compiled = bitlex_compile::compile(&stmts, &["letter", "scalar", "quote"]).expect("compiles");
    let tables = compiled.rows.lines().filter(|l| l.contains(" table ")).count();
    // Four products: one table.
    assert_eq!(tables, 1, "{}", compiled.rows);
    assert!(compiled.rows.contains("= eq 0x22"), "{}", compiled.rows);
}

/// What the compiler does not handle yet is refused by name, not guessed.
#[test]
fn unsupported_operations_are_refused() {
    let stmts = ::bitlex::syntax::parse("class a = 'a'\nmask b = next(a)\n").expect("parses");
    let error = bitlex_compile::compile(&stmts, &["b"]).expect_err("refused");
    assert!(error.contains("next"), "{error}");
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
    let compiled = bitlex_compile::compile(&stmts, &names).unwrap_or_else(|e| panic!("compiles: {e}"));
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
