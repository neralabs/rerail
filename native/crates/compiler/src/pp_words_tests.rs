//! The preprocessing words' verifier (`pp_words::validate`).

use crate::program::*;
use crate::rir_asm::assemble;

fn program(words: &str) -> Result<Words, String> {
    assemble(&format!("program - ir=7 grammar=none\n  string 0 \"(\"\n{words}"))
}

#[test]
fn a_copy_loop_is_accepted() {
    let p = program("  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 PUT 0\n  0004 GOTO @0000\n").unwrap();
    assert!(p.words.iter().any(|&w| crate::pp_words::is_pp(split(w).0)));
}

#[test]
fn a_cycle_that_reads_no_token_is_refused() {
    let e = program("  0000 PUT 0\n  0001 GOTO @0000\n").err().unwrap();
    assert!(e.contains("NonProgressingCycle"), "{e}");
    // A call whose callee can return without a read reads nothing either.
    let e = program("  0000 PUSH @0002\n  0001 GOTO @0004\n  0002 PUT 0\n  0003 GOTO @0000\n  0004 RETURN 0\n").err().unwrap();
    assert!(e.contains("NonProgressingCycle"), "{e}");
    let ok = program("  0000 PUSH @0002\n  0001 GOTO @0004\n  0002 PUT 0\n  0003 GOTO @0000\n  0004 READ 0\n  0005 RETURN 0\n");
    assert!(ok.as_ref().err().is_none(), "{:?}", ok.err());
}

#[test]
fn bad_operands_and_falling_off_the_end_are_refused() {
    for words in ["  0000 IS m=0 9\n  0001 GOTO @0000\n  0002 READ 0\n  0003 GOTO @0000\n", "  0000 PEEK m=1 s1\n  0001 READ 0\n  0002 GOTO @0000\n", "  0000 READ 0\n", "  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n"] {
        assert!(program(words).is_err(), "{words}");
    }
}
