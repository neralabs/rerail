//! Regions (`regions H { … }`, decision 0139): the rounds a backend runs (Metal's) are the walk
//! the CPU runs.

use crate::gating_tests::small;

/// Two kinds whose openers share a byte: the walk opens the first-listed kind there, so the rounds
/// must too (a raw string's `"(` is no plain string's `"`).
#[test]
fn a_byte_two_kinds_open_is_the_first_kinds() {
    let g = small(
        "lexer t
class dq = '\"'
class r  = 'R'
class lp = '('
class rp = ')'
mask raw_open  = dq & prev(r) & next(lp)
mask raw_close = dq & prev(rp)
regions hidden {
  raw = raw_open .. raw_close
  str = dq .. dq
}
escape \"both\" when o_raw & o_str
",
    );
    let files: [&[u8]; 4] = [b"a R\"(x\"y)\" b \"s\" R\"(\")\"", b"R\"()\"", b"\"R\"(\"", b"x R\"( open"];
    let (walked, _) = g.block.all_values(&files);
    let (rounds, _) = g.block.rounds_values(&files);
    for (v, (w, r)) in walked.iter().zip(&rounds).enumerate() {
        assert_eq!(w, r, "value {v}: walked against rounds");
    }
}
