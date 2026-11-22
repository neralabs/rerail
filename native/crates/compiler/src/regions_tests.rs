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

/// An opener of three bytes (`O bytes 3`): its closer is found past them, as the lean lexer
/// looks for a raw string's closer from its `"""`'s end, so `""""` opens and does not close.
/// The rounds and the walk agree, an opener in a block's last bytes too (the walk carries its
/// bytes over).
#[test]
fn a_closer_is_found_past_its_openers_bytes() {
    let g = small(
        "lexer t
class dq = '\"'
mask dq3   = dq & next(dq) & next(dq, 2)
regions hidden {
  raw = dq3 bytes 3 .. dq3 bytes 3
  str = dq .. dq
}
escape \"both\" when o_raw & o_str
class x = 'x'
escape \"a string\" when o_str
escape \"an x in a raw string\" when f_raw & x
escape \"a quote a raw string fills\" when f_raw & dq & prev(dq) & ~prev(o_raw)
",
    );
    let pad = |n: usize, s: &str| [vec![b'x'; n], s.as_bytes().to_vec()].concat();
    let mut files: Vec<Vec<u8>> = [&b"\"\"\"\"x\"\"\""[..], b"\"\"\"\"\"\" \"a\"", b"\"\"\"\"\"\"\" \"a\"", b"\"\"\"a\"\"\"\"", b"\"\"\"\"", b"x \"\"\" y"].iter().map(|f| f.to_vec()).collect();
    for n in 60..=64 {
        files.push(pad(n, "\"\"\"\"x\"\"\" \"s\""));
        files.push(pad(n, "\"\"\"\"\"\" \"s\""));
    }
    let files: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let (walked, _) = g.block.all_values(&files);
    let (rounds, _) = g.block.rounds_values(&files);
    for (v, (w, r)) in walked.iter().zip(&rounds).enumerate() {
        assert_eq!(w, r, "value {v}: walked against rounds");
    }
    // The JIT's walk (`native_cpu_walk`), file by file.
    let compiled = crate::native_cpu_block::compile(&g.native).expect("compiles");
    for f in &files {
        assert_eq!(compiled.run_files(&[f]), g.block.run_files(&[f]), "{:?}", String::from_utf8_lossy(f));
    }
}
