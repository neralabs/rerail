//! Tier 2's words (decision 0087), on hand-written programs: what they write, and what the
//! verifier refuses.

use super::*;

/// A root of kind 1 over a run of `a` tokens, each a leaf of kind 2: two states, the second a
/// loop through `TOKEN`.
const RUN: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 output %0
  block 0 extract %0
  block 0 mask %0
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

/// Tier 1's run as the words read it: the starts (extract 0) and last bytes (mask 0, output 0).
fn tier1(starts: &[u32], lasts: &[u32]) -> block::BlockRun {
    let mut words = vec![0u64; 1];
    for &l in lasts {
        words[l as usize / 64] |= 1 << (l % 64);
    }
    block::BlockRun { outputs: vec![words], positions: vec![starts.to_vec()], totals: Vec::new() }
}

#[test]
fn a_tier2_program_writes_its_nodes() {
    let p = rir::assemble(RUN).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rir::assemble(&rir::disassemble(&p, None)).expect("round trips").plain_digest(), p.plain_digest());
    let nodes = run(&p, b"aa a ", &tier1(&[0, 1, 3], &[])).expect("parses");
    let spans: Vec<(u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.start, n.end, n.size)).collect();
    assert_eq!(spans, [(1, 0, 5, 4), (2, 0, 1, 1), (2, 1, 2, 1), (2, 3, 4, 1)]);
    // A byte no state takes, a token past the file, an empty stream: refused.
    assert!(run(&p, b"ab", &tier1(&[0, 1], &[])).is_err());
    assert!(run(&p, b"a", &tier1(&[0, 1], &[])).is_err());
    assert!(run(&p, b"", &tier1(&[], &[])).is_err());
    // The last bytes are the mask a `SPAN m=1` names; this program names none, so it reads
    // none. One that does takes a last byte per token, the next bit after the last one taken,
    // and refuses one left unread.
    let lasts = rir::assemble(&RUN.replace("  0003 SPAN 1\n", "  0003 SPAN m=1 0\n")).unwrap_or_else(|e| panic!("{e}"));
    assert!(run(&lasts, b"aa", &tier1(&[0, 1], &[0, 1])).is_ok());
    assert!(run(&lasts, b"aa", &tier1(&[0, 1], &[0, 1, 2])).is_err());
    assert!(run(&lasts, b"aa", &tier1(&[0, 1], &[0])).is_err());
}

/// The verifier refuses a loop that does not pass a `TOKEN`: here the second switch jumps to
/// itself, so its loop is not over the stream.
#[test]
fn a_loop_not_over_the_stream_is_refused() {
    let looping = RUN.replace("switch 1 0x61-0x61 @0003", "switch 1 0x61-0x61 @0006");
    let e = rir::assemble(&looping).err().expect("a loop without a token");
    assert!(e.contains("NonProgressingCycle"), "{e}");
    // A `GOTO` back without a `TOKEN` on the way, likewise.
    let back = RUN.replace("  0007 ACCEPT 0\n", "  0007 GOTO @0003\n").replace("switch 1 at 0 end @0007", "switch 1 at 0 end @0008") + "  0008 ACCEPT 0\n";
    rir::assemble(&back).expect("a loop through a token is the stream's");
    let no_token = back.replace("  0005 TOKEN 0\n", "  0005 LEAF #2\n");
    assert!(rir::assemble(&no_token).err().expect("no token on the way").contains("NonProgressingCycle"));
}

/// A tier-2 program holds only tier 2's words, `STOP` and `GOTO`; a return lands on a state's
/// `TOKEN` and `SWITCH`.
#[test]
fn a_tier2_program_holds_only_its_words() {
    let with_child = RUN.replace("  0007 ACCEPT 0\n", "  0007 CHILD 0\n");
    assert!(rir::assemble(&with_child).is_err());
    let bad_return = RUN.replace("  0004 LEAF #2\n", "  0004 PUSH @0002\n");
    assert!(rir::assemble(&bad_return).is_err());
}

/// A root over `a(`: the `(` wraps the `a` leaf made before it (`WRAP`): the wrap takes the
/// leaf's place, field and start, and the leaf gets its own field and kind (`NODE_FIELD m=1`,
/// `RETAG m=1`). The wrap's record is made after the leaf's; at `ACCEPT` it goes before it.
const WRAPPED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 output %0
  block 0 extract %0
  block 0 mask %0
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
    let p = rir::assemble(WRAPPED).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rir::assemble(&rir::disassemble(&p, None)).expect("round trips").plain_digest(), p.plain_digest());
    let t1 = tier1(&[0, 1], &[]);
    let nodes = run(&p, b"a(", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; the wrap, from the leaf's start; the leaf, now kind 4 with field 5; the `(`.
    let want = [(1, 0, 0, 2, 4), (3, 0, 0, 2, 3), (4, 5, 0, 1, 1), (6, 0, 1, 2, 1)];
    assert_eq!(got, want);
    let native = native_ir::lower(&p).expect("lowers");
    let compiled = native_cpu_stream::compile(&native).expect("compiles");
    assert!(compiled.wraps, "the tape has the wrap's columns");
    assert_eq!(compiled.run(b"a(", &t1).expect("parses"), nodes);
    // A wrap with no node made before it at its level: refused.
    let first = WRAPPED.replace("  0004 LEAF #2\n", "  0004 RETAG #1\n");
    assert!(run(&rir::assemble(&first).expect("assembles"), b"a(", &t1).is_err());
}

/// A root with a leaf, then `(`: the `(` relabels the leaf (`NODE_FIELD m=2`, `RETAG m=2`, the
/// node made last at the root's level), with no node around it.
const RELABELLED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 output %0
  block 0 extract %0
  block 0 mask %0
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
    let p = rir::assemble(RELABELLED).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rir::assemble(&rir::disassemble(&p, None)).expect("round trips").plain_digest(), p.plain_digest());
    let t1 = tier1(&[0, 1], &[]);
    let nodes = run(&p, b"a(", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; the leaf, now kind 4 with field 5; the `(`.
    assert_eq!(got, [(1, 0, 0, 2, 3), (4, 5, 0, 1, 1), (6, 0, 1, 2, 1)]);
    let native = native_ir::lower(&p).expect("lowers");
    let compiled = native_cpu_stream::compile(&native).expect("compiles");
    assert!(compiled.wraps, "the tape has the frames");
    assert_eq!(compiled.run(b"a(", &t1).expect("parses"), nodes);
    let first = RELABELLED.replace("  0004 LEAF #2\n", "  0004 RETAG #1\n");
    let first = rir::assemble(&first).expect("assembles");
    assert!(run(&first, b"a(", &t1).is_err());
    let compiled = native_cpu_stream::compile(&native_ir::lower(&first).expect("lowers")).expect("compiles");
    assert!(compiled.run(b"a(", &t1).is_err());
}

/// `a(b`: a wrap around `a` at `(`, closed at `b`; then `b` relabels it (`RETAG m=2`) and the
/// node it holds first (`NODE_FIELD m=3`): an atom whose word turns out a call's.
const RELABELLED_INNER: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 output %0
  block 0 extract %0
  block 0 mask %0
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
    let p = rir::assemble(RELABELLED_INNER).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(&[0, 1, 2], &[]);
    let nodes = run(&p, b"a(b", &t1).expect("parses");
    let got: Vec<(u16, u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.field, n.start, n.end, n.size)).collect();
    // The root; the wrap, now kind 8; `a`, field 7 (was 5); the `(`; the `b`.
    assert_eq!(got, [(1, 0, 0, 3, 5), (8, 0, 0, 2, 3), (2, 7, 0, 1, 1), (6, 0, 1, 2, 1), (9, 0, 2, 3, 1)]);
    let compiled = native_cpu_stream::compile(&native_ir::lower(&p).expect("lowers")).expect("compiles");
    assert_eq!(compiled.run(b"a(b", &t1).expect("parses"), nodes);
    // No wrap: the leaf `a` is the node made last.
    let plain = RELABELLED_INNER.replace("  0008 WRAP #3\n", "  0008 NODE_SUPER #3\n").replace("  0014 CLOSE 0\n", "  0014 LEAF #6\n");
    let plain = rir::assemble(&plain).expect("assembles");
    assert!(run(&plain, b"a(b", &t1).is_err());
    let compiled = native_cpu_stream::compile(&native_ir::lower(&plain).expect("lowers")).expect("compiles");
    assert!(compiled.run(b"a(b", &t1).is_err());
}

/// A root over `a a a`, each `a` after the first a call into a level that ends on the token
/// after: at the stream's end the levels return one after another, each to switch on the end
/// again (`RETURN m=1`), with no `TOKEN` between.
const CHAINED: &str = "program -
  mode set=0 query=0
  block 0 %0 = eq 0x61
  block 0 output %0
  block 0 extract %0
  block 0 mask %0
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
    let p = rir::assemble(CHAINED).unwrap_or_else(|e| panic!("{e}"));
    let t1 = tier1(&[0, 2, 4], &[]);
    let nodes = run(&p, b"a a a", &t1).expect("parses");
    let got: Vec<(u16, u32, u32, u32)> = nodes.iter().map(|n| (n.kind, n.start, n.end, n.size)).collect();
    assert_eq!(got, [(1, 0, 5, 4), (2, 0, 1, 1), (2, 2, 3, 1), (2, 4, 5, 1)]);
    let native = native_ir::lower(&p).expect("lowers");
    let compiled = native_cpu_stream::compile(&native).expect("compiles: the return's block is a named loop head");
    assert!(compiled.heads.iter().any(|h| h.1 == native_cpu_shape::Step::Return), "the return is a loop's head");
    assert_eq!(compiled.run(b"a a a", &t1).expect("parses"), nodes);
    let popped = CHAINED.replace("  0014 GOTO @0009\n", "  0014 GOTO @0015\n");
    let e = rir::assemble(&popped).err().expect("a call popped at once");
    assert!(e.contains("NonProgressingCycle"), "{e}");
}
