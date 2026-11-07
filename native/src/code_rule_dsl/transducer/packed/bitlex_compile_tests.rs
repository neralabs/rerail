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

/// Of a run of variants the compiler keeps the first; its `unless` mask is the fallback bit. On a
/// file the evaluator runs it on, its masks are equal and the fallback bit is clear; on any other,
/// the fallback bit is set. The second variant has a machine, which is not compiled.
const VARIANTS: &str = r#"
class quote     = '"'
class backslash = '\\'
class slash     = '/'
mask escaped    = after_odd_run(backslash)
variant plain {
  mask inside   = toggle(quote & ~escaped)
} unless slash & ~inside
variant general {
  machine m {
    events {
      q = quote & ~escaped
    }
    states out in
    start out
    final out
    out on q -> in
    in  on q -> out
  }
  mask inside   = m.in(in)
}
mask code       = ~inside
"#;

#[test]
fn the_first_variant_is_compiled_and_its_test_is_the_fallback() {
    let names = ["inside", "code", bitlex_compile::FALLBACK];
    let stmts = ::bitlex::syntax::parse(VARIANTS).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &names).unwrap_or_else(|e| panic!("compiles: {e}"));
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let mut cases = inputs();
    cases.extend([b"{\"a/b\": 1}".to_vec(), b"{\"a\": 1} // c".to_vec(), b"/".to_vec()]);
    let (mut plain, mut general) = (0, 0);
    for bytes in cases {
        let tables = std::collections::HashMap::new();
        let out = ::bitlex::eval::Ev::run(&stmts, &bytes, &tables).expect("evaluates");
        let kept_plain = out.variants == ["plain"];
        let want = evaluated(&stmts, &bytes, &names[..2]);
        for (executor, got) in executors(&p, &bytes) {
            let fallback = cut(got[2].clone(), bytes.len()).iter().any(|&w| w != 0);
            assert_eq!(fallback, !kept_plain, "{executor}, {} bytes", bytes.len());
            if kept_plain {
                for k in 0..2 {
                    assert_eq!(cut(got[k].clone(), bytes.len()), want[k], "{}, {executor}, {} bytes", names[k], bytes.len());
                }
            }
        }
        if kept_plain { plain += 1 } else { general += 1 }
    }
    assert!(plain > 0 && general > 0, "both variants reached: {plain} plain, {general} general");
}
