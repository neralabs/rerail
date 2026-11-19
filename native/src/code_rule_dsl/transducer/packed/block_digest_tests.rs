//! A program's block programs are in its digest: two programs that differ only in their block
//! program, or in having one, digest differently. (The block programs' own tests are the parser
//! tests crate's, `rerail_parser_tests::block_tests`.)

use super::rir;

fn program(rows: &str) -> super::Program {
    let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
    rir::assemble(&text).unwrap_or_else(|error| panic!("{error}\n{text}"))
}

#[test]
fn block_programs_are_in_the_digest() {
    let p = program("  block 0 %0 = eq 0x22\n  block 0 output %0\n");
    let again = rir::assemble(&rir::disassemble(&p, None)).expect("the dump assembles");
    assert_eq!(again.plain_digest(), p.plain_digest());
    let other = program("  block 0 %0 = eq 0x5c\n  block 0 output %0\n");
    assert_ne!(other.plain_digest(), p.plain_digest());
    let none = rir::assemble("program -\n  mode set=0 query=0\n  0000 STOP 0\n").unwrap();
    assert_ne!(none.plain_digest(), p.plain_digest());
}
