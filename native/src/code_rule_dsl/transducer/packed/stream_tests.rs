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
