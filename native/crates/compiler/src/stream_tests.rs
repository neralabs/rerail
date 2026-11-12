//! Tier 2's words (decision 0087), on hand-written programs: what they write, and what the
//! verifier refuses. Each round trips through the
//! disassembler (`rir_asm::disassemble`) to the same text.

use crate::stream::*;

/// A root of kind 1 over a run of `a` tokens, each a leaf of kind 2: two states, the second a
/// loop through `TOKEN`.
const RUN: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 %1 = const 0x0000000000000000
  block 0 output %0
  block 0 output %1
  block 0 extract %0
  block 0 extract %0
  block 0 total %1
  block 0 records 0 1 kinds=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
  switch 0 at 0 end refuse
  switch 0 0x61-0x61 @0002
  switch 1 at 0 end @0007
  switch 1 0x61-0x61 @0003
  0000 TOKEN 0
  0001 SWITCH T0
  0002 OPEN #1
  0003 SPAN 1
  0004 LEAF #2
  0005 TOKEN 0
  0006 SWITCH T1
  0007 ACCEPT 0
";

/// Tier 1's run as the words read it: the records (`records`), the starts (extract 0) and
/// last bytes (extract 1, the starts where none are given: one-byte tokens), each code its first
/// byte (the programs' kinds are their bytes; no words).
fn tier1(bytes: &[u8], starts: &[u32], lasts: &[u32]) -> crate::block::BlockRun {
    let lasts = if lasts.is_empty() { starts } else { lasts };
    let n = starts.len().min(lasts.len());
    let codes = starts[..n].iter().map(|&s| u32::from(bytes.get(s as usize).copied().unwrap_or(0))).collect();
    crate::block::BlockRun { outputs: vec![vec![0], vec![0]], positions: vec![starts.to_vec(), lasts.to_vec()], totals: vec![0], codes }
}

#[test]
fn a_tier2_program_writes_its_nodes() {
    let p = crate::rir_asm::assemble(RUN).unwrap_or_else(|e| panic!("{e}"));
    let nodes = run(&p, b"aa a ", &tier1(b"aa a ", &[0, 1, 3], &[])).expect("parses");
    let spans: Vec<(u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.start, n.end, n.size)).collect();
    assert_eq!(spans, [(1, 0, 5, 4), (2, 0, 1, 1), (2, 1, 2, 1), (2, 3, 4, 1)]);
    // A byte no state takes, a token past the file, an empty stream: refused.
    assert!(run(&p, b"ab", &tier1(b"ab", &[0, 1], &[])).is_err());
    assert!(run(&p, b"a", &tier1(b"a", &[0, 1], &[])).is_err());
    assert!(run(&p, b"", &tier1(b"", &[], &[])).is_err());
    // A token's end is its record's (`SPAN m=1`); a length (`SPAN 1`) must be the record's.
    let lasts = crate::rir_asm::assemble(&RUN.replace("  0003 SPAN 1\n", "  0003 SPAN m=1 0\n")).unwrap_or_else(|e| panic!("{e}"));
    assert!(run(&lasts, b"aa", &tier1(b"aa", &[0, 1], &[0, 1])).is_ok());
    assert!(run(&p, b"aa", &tier1(b"aa", &[0, 1], &[0, 1])).is_ok());
    assert!(run(&p, b"aa a", &tier1(b"aa a", &[0, 3], &[1, 3])).is_err(), "a length not the record's");
    // The record pass refuses streams that disagree: as many lasts as starts, each last in its
    // token, each token after the one before.
    let block = &p.block_programs[0];
    let table = crate::records::WordTable::new(&[]).expect("no words");
    let r = block.records.as_ref().expect("records");
    let bad = |s: &[u32], l: &[u32]| crate::records::pass(r, &table, b"aaaa", s, l).1;
    assert!(!bad(&[0, 1], &[0, 1]));
    assert!(bad(&[0, 1], &[0, 1, 2]));
    assert!(bad(&[0, 1], &[0]));
    assert!(bad(&[1, 2], &[0, 2]), "a last before its token");
    assert!(bad(&[0, 1], &[1, 2]), "a token inside the one before");
}

/// The verifier refuses a loop that does not pass a `TOKEN`: here the second switch jumps to
/// itself, so its loop is not over the stream.
#[test]
fn a_loop_not_over_the_stream_is_refused() {
    let looping = RUN.replace("switch 1 0x61-0x61 @0003", "switch 1 0x61-0x61 @0006");
    let e = crate::rir_asm::assemble(&looping).err().expect("a loop without a token");
    assert!(e.contains("NonProgressingCycle"), "{e}");
    // A `GOTO` back without a `TOKEN` on the way, likewise.
    let back = RUN.replace("  0007 ACCEPT 0\n", "  0007 GOTO @0003\n").replace("switch 1 at 0 end @0007", "switch 1 at 0 end @0008") + "  0008 ACCEPT 0\n";
    crate::rir_asm::assemble(&back).expect("a loop through a token is the stream's");
    let no_token = back.replace("  0005 TOKEN 0\n", "  0005 LEAF #2\n");
    assert!(crate::rir_asm::assemble(&no_token).err().expect("no token on the way").contains("NonProgressingCycle"));
}

/// A tier-2 program holds only tier 2's words, `STOP` and `GOTO`; a return lands on a state's
/// `TOKEN` and `SWITCH`.
#[test]
fn a_tier2_program_holds_only_its_words() {
    let with_child = RUN.replace("  0007 ACCEPT 0\n", "  0007 CHILD 0\n");
    assert!(crate::rir_asm::assemble(&with_child).is_err());
    let bad_return = RUN.replace("  0004 LEAF #2\n", "  0004 PUSH @0002\n");
    assert!(crate::rir_asm::assemble(&bad_return).is_err());
}

/// A root over `a(`: the `(` wraps the `a` leaf made before it (`WRAP`): the wrap takes the
/// leaf's place, field and start, and the leaf gets its own field and kind (`NODE_FIELD m=1`,
/// `RETAG m=1`). The wrap's record is made after the leaf's; at `ACCEPT` it goes before it.
const WRAPPED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 %1 = const 0x0000000000000000
  block 0 output %0
  block 0 output %1
  block 0 extract %0
  block 0 extract %0
  block 0 total %1
  block 0 records 0 1 kinds=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
  switch 0 at 0 end refuse
  switch 0 0x61-0x61 @0002
  switch 1 at 0 end refuse
  switch 1 0x28-0x28 @0007
  switch 2 at 0 end @0014
  0000 TOKEN 0
  0001 SWITCH T0
  0002 OPEN #1
  0003 SPAN 1
  0004 LEAF #2
  0005 TOKEN 0
  0006 SWITCH T1
  0007 SPAN 1
  0008 WRAP #3
  0009 NODE_FIELD m=1 #5
  0010 RETAG m=1 #4
  0011 LEAF #6
  0012 TOKEN 0
  0013 SWITCH T2
  0014 CLOSE 0
  0015 ACCEPT 0
";

/// The wrap's nodes, in preorder, from the interpreter and from Cranelift.
#[test]
fn a_wrap_goes_before_the_node_it_wraps() {
    let p = crate::rir_asm::assemble(WRAPPED).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(b"a(", &[0, 1], &[]);
    let nodes = run(&p, b"a(", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; the wrap, from the leaf's start; the leaf, now kind 4 with field 5; the `(`.
    let want = [(1, 0, 0, 2, 4), (3, 0, 0, 2, 3), (4, 5, 0, 1, 1), (6, 0, 1, 2, 1)];
    assert_eq!(got, want);
    let native = crate::native_ir::lower(&p, 0, 0).expect("lowers");
    let compiled = crate::native_cpu_stream::compile(&native).expect("compiles");
    assert!(compiled.wraps, "the tape has the wrap's columns");
    assert_eq!(compiled.run(b"a(", &t1).expect("parses"), nodes);
    // A wrap with no node made before it at its level: refused.
    let first = WRAPPED.replace("  0004 LEAF #2\n", "  0004 RETAG #1\n");
    assert!(run(&crate::rir_asm::assemble(&first).expect("assembles"), b"a(", &t1).is_err());
}

/// A root with a leaf, then `(`: the `(` relabels the leaf (`NODE_FIELD m=2`, `RETAG m=2`, the
/// node made last at the root's level), with no node around it.
const RELABELLED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 %1 = const 0x0000000000000000
  block 0 output %0
  block 0 output %1
  block 0 extract %0
  block 0 extract %0
  block 0 total %1
  block 0 records 0 1 kinds=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
  switch 0 at 0 end refuse
  switch 0 0x61-0x61 @0002
  switch 1 at 0 end refuse
  switch 1 0x28-0x28 @0007
  switch 2 at 0 end @0013
  0000 TOKEN 0
  0001 SWITCH T0
  0002 OPEN #1
  0003 SPAN 1
  0004 LEAF #2
  0005 TOKEN 0
  0006 SWITCH T1
  0007 SPAN 1
  0008 NODE_FIELD m=2 #5
  0009 RETAG m=2 #4
  0010 LEAF #6
  0011 TOKEN 0
  0012 SWITCH T2
  0013 ACCEPT 0
";

/// The relabelled leaf, from the interpreter and from Cranelift; with no node made at the level
/// before it, refused by both.
#[test]
fn a_relabel_acts_on_the_node_made_last() {
    let p = crate::rir_asm::assemble(RELABELLED).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(b"a(", &[0, 1], &[]);
    let nodes = run(&p, b"a(", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; the leaf, now kind 4 with field 5; the `(`.
    assert_eq!(got, [(1, 0, 0, 2, 3), (4, 5, 0, 1, 1), (6, 0, 1, 2, 1)]);
    let native = crate::native_ir::lower(&p, 0, 0).expect("lowers");
    let compiled = crate::native_cpu_stream::compile(&native).expect("compiles");
    assert!(compiled.wraps, "the tape has the frames");
    assert_eq!(compiled.run(b"a(", &t1).expect("parses"), nodes);
    let first = RELABELLED.replace("  0004 LEAF #2\n", "  0004 RETAG #1\n");
    let first = crate::rir_asm::assemble(&first).expect("assembles");
    assert!(run(&first, b"a(", &t1).is_err());
    let compiled = crate::native_cpu_stream::compile(&crate::native_ir::lower(&first, 0, 0).expect("lowers")).expect("compiles");
    assert!(compiled.run(b"a(", &t1).is_err());
}

/// `ab(`: the root's two leaves, `a` and `b`, become one at `(` (`MERGE m=1`, `fold`).
const FOLDED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 %1 = const 0x0000000000000000
  block 0 output %0
  block 0 output %1
  block 0 extract %0
  block 0 extract %0
  block 0 total %1
  block 0 records 0 1 kinds=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
  switch 0 at 0 end refuse
  switch 0 0x61-0x61 @0002
  switch 1 at 0 end refuse
  switch 1 0x62-0x62 @0007
  switch 2 at 0 end refuse
  switch 2 0x28-0x28 @0011
  switch 3 at 0 end @0017
  0000 TOKEN 0
  0001 SWITCH T0
  0002 OPEN #1
  0003 SPAN 1
  0004 LEAF #2
  0005 TOKEN 0
  0006 SWITCH T1
  0007 SPAN 1
  0008 LEAF #3
  0009 TOKEN 0
  0010 SWITCH T2
  0011 SPAN 1
  0012 MERGE m=1 #4
  0013 LEAF #6
  0014 TOKEN 0
  0015 SWITCH T3
  0016 STOP 0
  0017 ACCEPT 0
";

/// The folded leaf, from the interpreter and from Cranelift, framed or not; with a third node
/// in the root, or a pending extra, refused by both.
#[test]
fn a_fold_makes_the_open_nodes_two_leaves_one() {
    let p = crate::rir_asm::assemble(FOLDED).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(b"ab(", &[0, 1, 2], &[]);
    let nodes = run(&p, b"ab(", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; `ab`, one leaf of kind 4; the `(`.
    assert_eq!(got, [(1, 0, 0, 3, 3), (4, 0, 0, 2, 1), (6, 0, 2, 3, 1)]);
    let compiled = crate::native_cpu_stream::compile(&crate::native_ir::lower(&p, 0, 0).expect("lowers")).expect("compiles");
    assert!(!compiled.wraps);
    assert_eq!(compiled.run(b"ab(", &t1).expect("parses"), nodes);
    // Framed (a relabel elsewhere keeps the frames): the same nodes.
    let framed = FOLDED.replace("  0016 STOP 0\n", "  0016 RETAG m=2 #4\n");
    let framed = crate::rir_asm::assemble(&framed).expect("assembles");
    let compiled = crate::native_cpu_stream::compile(&crate::native_ir::lower(&framed, 0, 0).expect("lowers")).expect("compiles");
    assert!(compiled.wraps, "the tape has the frames");
    assert_eq!(compiled.run(b"ab(", &t1).expect("parses"), nodes);
    for bad in [
        FOLDED.replace("  0004 LEAF #2\n", "  0004 OPEN #2\n"),
        FOLDED.replace("  0008 LEAF #3\n", "  0008 LEAF m=1 #3\n"),
        FOLDED.replace("  0012 MERGE m=1 #4\n", "  0012 LEAF #5\n").replace("  0013 LEAF #6\n", "  0013 MERGE m=1 #4\n"),
    ] {
        let bad = crate::rir_asm::assemble(&bad).expect("assembles");
        assert!(run(&bad, b"ab(", &t1).is_err());
        let compiled = crate::native_cpu_stream::compile(&crate::native_ir::lower(&bad, 0, 0).expect("lowers")).expect("compiles");
        assert!(compiled.run(b"ab(", &t1).is_err());
    }
}

/// `a(b`: a wrap around `a` at `(`, closed at `b`; then `b` relabels it (`RETAG m=2`) and the
/// node it holds first (`NODE_FIELD m=3`): an atom whose word turns out a call's.
const RELABELLED_INNER: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 %1 = const 0x0000000000000000
  block 0 output %0
  block 0 output %1
  block 0 extract %0
  block 0 extract %0
  block 0 total %1
  block 0 records 0 1 kinds=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
  switch 0 at 0 end refuse
  switch 0 0x61-0x61 @0002
  switch 1 at 0 end refuse
  switch 1 0x28-0x28 @0007
  switch 2 at 0 end refuse
  switch 2 0x62-0x62 @0013
  switch 3 at 0 end @0020
  0000 TOKEN 0
  0001 SWITCH T0
  0002 OPEN #1
  0003 SPAN 1
  0004 LEAF #2
  0005 TOKEN 0
  0006 SWITCH T1
  0007 SPAN 1
  0008 WRAP #3
  0009 NODE_FIELD m=1 #5
  0010 LEAF #6
  0011 TOKEN 0
  0012 SWITCH T2
  0013 SPAN 1
  0014 CLOSE 0
  0015 NODE_FIELD m=3 #7
  0016 RETAG m=2 #8
  0017 LEAF #9
  0018 TOKEN 0
  0019 SWITCH T3
  0020 ACCEPT 0
";

/// The inner field on the wrap's first node, from the interpreter and from Cranelift; on a
/// node that is no wrap, refused by both.
#[test]
fn an_inner_relabel_acts_on_the_node_a_wrap_holds_first() {
    let p = crate::rir_asm::assemble(RELABELLED_INNER).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(b"a(b", &[0, 1, 2], &[]);
    let nodes = run(&p, b"a(b", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; the wrap, now kind 8; `a`, field 7 (was 5); the `(`; the `b`.
    assert_eq!(got, [(1, 0, 0, 3, 5), (8, 0, 0, 2, 3), (2, 7, 0, 1, 1), (6, 0, 1, 2, 1), (9, 0, 2, 3, 1)]);
    let compiled = crate::native_cpu_stream::compile(&crate::native_ir::lower(&p, 0, 0).expect("lowers")).expect("compiles");
    assert_eq!(compiled.run(b"a(b", &t1).expect("parses"), nodes);
    // No wrap: the leaf `a` is the node made last.
    let plain = RELABELLED_INNER.replace("  0008 WRAP #3\n", "  0008 NODE_SUPER #3\n").replace("  0014 CLOSE 0\n", "  0014 LEAF #6\n");
    let plain = crate::rir_asm::assemble(&plain).expect("assembles");
    assert!(run(&plain, b"a(b", &t1).is_err());
    let compiled = crate::native_cpu_stream::compile(&crate::native_ir::lower(&plain, 0, 0).expect("lowers")).expect("compiles");
    assert!(compiled.run(b"a(b", &t1).is_err());
}

/// A root over `a a a`, each `a` after the first a call into a level that ends on the token
/// after: at the stream's end the levels return one after another, each to switch on the end
/// again (`RETURN m=1`), with no `TOKEN` between.
const CHAINED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 %1 = const 0x0000000000000000
  block 0 output %0
  block 0 output %1
  block 0 extract %0
  block 0 extract %0
  block 0 total %1
  block 0 records 0 1 kinds=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
  switch 0 at 0 end refuse
  switch 0 0x61-0x61 @0002
  switch 1 at 0 end @0016
  switch 2 at 0 end @0015
  switch 2 0x61-0x61 @0011
  0000 TOKEN 0
  0001 SWITCH T0
  0002 OPEN #1
  0003 SPAN 1
  0004 LEAF #2
  0005 PUSH @0007
  0006 GOTO @0009
  0007 TOKEN 0
  0008 SWITCH T1
  0009 TOKEN 0
  0010 SWITCH T2
  0011 SPAN 1
  0012 LEAF #2
  0013 PUSH @0009
  0014 GOTO @0009
  0015 RETURN m=1 0
  0016 ACCEPT 0
";

/// Returns may chain: a cycle through a `RETURN` with no `TOKEN` is allowed, as each turn pops
/// a call, and the calls are made between tokens. A `RETURN` reached from a `PUSH` before any
/// `TOKEN` is refused: a call made and popped with no token between could turn without end.
#[test]
fn returns_chain_but_a_call_is_never_popped_at_once() {
    let p = crate::rir_asm::assemble(CHAINED).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(b"a a a", &[0, 2, 4], &[]);
    let nodes = run(&p, b"a a a", &t1).expect("parses");
    let got: Vec<(u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.start, n.end, n.size)).collect();
    assert_eq!(got, [(1, 0, 5, 4), (2, 0, 1, 1), (2, 2, 3, 1), (2, 4, 5, 1)]);
    let native = crate::native_ir::lower(&p, 0, 0).expect("lowers");
    let compiled = crate::native_cpu_stream::compile(&native).expect("compiles: the return's block is a named loop head");
    assert!(compiled.heads.iter().any(|h| h.1 == crate::native_cpu_shape::Step::Return), "the return is a loop's head");
    assert_eq!(compiled.run(b"a a a", &t1).expect("parses"), nodes);
    // A function a rule (`by_rules`): the calls native, the chain a return after a return.
    let cut = crate::native_cpu_clusters::by_rules(&native);
    assert!(cut.calls && cut.count > 1, "{cut:?}");
    let rules = crate::native_cpu_stream::compile_cut(&native, &cut).expect("compiles a function a rule");
    assert_eq!(rules.run(b"a a a", &t1).expect("parses"), nodes);
    assert_eq!(rules.run(b"a a", &tier1(b"a a", &[0, 2], &[])).map_err(|r| r.pc), compiled.run(b"a a", &tier1(b"a a", &[0, 2], &[])).map_err(|r| r.pc));
    let popped = CHAINED.replace("  0014 GOTO @0009\n", "  0014 GOTO @0015\n");
    let e = crate::rir_asm::assemble(&popped).err().expect("a call popped at once");
    assert!(e.contains("NonProgressingCycle"), "{e}");
}

/// Each program, disassembled and assembled again, prints the same: the disassembler is the
/// assembler's inverse on tier 2's rows and words.
#[test]
fn tier2_programs_round_trip_through_the_disassembler() {
    for text in [RUN, WRAPPED, RELABELLED, FOLDED, RELABELLED_INNER, CHAINED] {
        let first = crate::rir_asm::disassemble(&crate::rir_asm::assemble(text).unwrap_or_else(|e| panic!("{e}")));
        let again = crate::rir_asm::disassemble(&crate::rir_asm::assemble(&first).unwrap_or_else(|e| panic!("{e}\n{first}")));
        assert_eq!(again, first);
        assert!(first.contains(" TOKEN ") && first.contains("  switch 0 at "), "{first}");
    }
}
