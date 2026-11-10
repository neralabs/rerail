//! The fallback as one accumulated flag (doc 26 §1.5, transformation 4): simdjson ORs every
//! error into one word and tests it once. The fallback's masks stay at their own delays, one
//! output each (none brought to another by a `prev`: measured, that costs more carried bits
//! and spills than it saves); every executor ORs them all into one total per file, a single
//! running word in the loop, and tier 2 tests it with one `EMPTY`.

use crate::bitlex_masks::{self, FALLBACK};
use crate::block_native::BlockNative;
use crate::native_cpu_block;

/// Escapes at delays 0, 2 and 5 (each `next` delays its mask by its reach).
const DELAYS: &str = "lexer t
class a = 'a'
class b = 'b'
class c = 'c'
mask ea = a
mask eb = b & next(a, 2)
mask ec = c & next(b, 5)
escape \"a\" when ea
escape \"b\" when eb
escape \"c\" when ec
";

fn compiled(text: &str) -> bitlex_masks::Compiled {
    let stmts = ::bitlex::syntax::parse(text).expect("parses");
    bitlex_masks::compile(&stmts, &[FALLBACK], None).unwrap_or_else(|e| panic!("compiles: {e}"))
}

#[test]
fn the_fallbacks_masks_keep_their_delays() {
    let c = compiled(DELAYS);
    let delays: Vec<u8> = c.fallbacks.iter().map(|&k| c.program.output_delays[usize::from(k)]).collect();
    assert_eq!(delays, [0, 2, 5]);
}

/// The fallback's outputs as totals: the program `DELAYS` compiles to, its fallbacks totals.
fn with_totals() -> crate::block::BlockProgram {
    let c = compiled(DELAYS);
    let mut p = c.program;
    p.totals = c.fallbacks.iter().map(|&k| p.outputs[usize::from(k)]).collect();
    p
}

#[test]
fn every_total_is_one_flag() {
    let p = with_totals();
    let native = BlockNative::lower(&p);
    assert_eq!(native_cpu_block::counted(&native).len(), 1, "one running word for the totals");
    let cranelift = native_cpu_block::compile(&native).expect("compiles");
    for (text, want) in [(&b""[..], 0), (b"xxxx", 0), (b"a", 1), (b"bxa", 1), (b"cxxxxb", 1), (b"xbxxc", 0)] {
        let (interpreted, compiled) = (p.run(text), cranelift.extract(text));
        assert_eq!(interpreted.totals, [want], "the interpreter over {:?}", String::from_utf8_lossy(text));
        assert_eq!(compiled.totals, [want], "Cranelift over {:?}", String::from_utf8_lossy(text));
    }
}

/// JSON's tree program: tier 2 tests the totals once.
#[test]
fn a_tree_program_tests_its_totals_once() {
    let json = include_str!("../../../../docs/parser-generators/lexers/json.bitlex");
    let c = crate::ladder::compile_step("json", json);
    let text = crate::rir_asm::disassemble(&c.program);
    let empties = text.lines().filter(|l| l.split_whitespace().nth(1) == Some("EMPTY")).count();
    assert_eq!(empties, 1, "{text}");
}
