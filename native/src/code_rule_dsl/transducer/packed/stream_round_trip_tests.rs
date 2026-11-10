//! Tier 2's hand-written programs (`rerail_compiler::stream_tests`) through the engine's
//! assembler and disassembler: each round trips to the same digest.

use rerail_compiler::stream_tests::{RUN, WRAPPED, RELABELLED, FOLDED, RELABELLED_INNER, CHAINED};

#[test]
fn tier2_programs_round_trip_through_the_disassembler() {
    for text in [RUN, WRAPPED, RELABELLED, FOLDED, RELABELLED_INNER, CHAINED] {
        let p = super::super::rir::assemble(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(super::super::rir::assemble(&super::super::rir::disassemble(&p, None)).expect("round trips").plain_digest(), p.plain_digest());
    }
}
