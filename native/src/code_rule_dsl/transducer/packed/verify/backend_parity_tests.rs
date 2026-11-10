//! F1 (00-requirements): no Metal → CPU fallback, so Metal runs every program the verifier admits.
//! At the instruction level that is one opcode set: every opcode `Program::validate` accepts is a
//! case of the Metal interpreter's step switch, and the switch has no other. The JIT lowers the
//! same set (`native_cpu`'s structural operations).
use super::*;

const OPCODES: [u8; 25] = [
    STOP, KIND, FIELD, FLAGS, TEXT, SIZE, ASSERT, FIRST, LAST, ABSENT_FIELD, SUPERTYPE, CHILD, NEXT,
    PARENT, DESCEND, FOLLOWING, SAVE_CURSOR, RESTORE_CURSOR, CAPTURE, REPEAT_BEGIN, REPEAT_BUMP,
    REPEAT_END, FORK, GOTO, EMIT,
];

#[test]
fn the_verifier_and_the_metal_interpreter_share_one_opcode_set() {
    let source = include_str!("../metal_batch_runtime.metal");
    let (_, rest) = source.split_once("/*OP_SWITCH_BEGIN*/switch(op){").expect("the step switch");
    let (switch, _) = rest.split_once("/*OP_SWITCH_END*/").expect("the step switch's end");
    let mut metal = std::collections::BTreeSet::new();
    for line in switch.lines().filter(|line| line.trim_start().starts_with("case ")) {
        let mut rest = line.trim_start();
        while let Some(case) = rest.strip_prefix("case ") {
            let (number, tail) = case.split_once("u:").expect("a numbered case");
            metal.insert(number.parse::<u8>().expect("an opcode"));
            rest = tail;
        }
    }
    let admitted: std::collections::BTreeSet<u8> = (0..=u8::MAX)
        .filter(|&opcode| {
            // A minimal program around the opcode; only the opcode's own checks can refuse it.
            let words = vec![word(opcode, 0, 0), word(STOP, 0, 0)];
            !matches!(
                Program::new(words, vec![b"value".to_vec()], vec![], vec![0], vec![]),
                Err(Error::InvalidOpcode(0))
            )
        })
        .collect();
    // Tier 2's words (decision 0087) are a program of their own, over the token stream. Metal
    // runs them in phase 6 (doc 17); until then they are the one known gap of F1, named here.
    let tier2: std::collections::BTreeSet<u8> = admitted.iter().copied().filter(|&o| stream::is_stream(o)).collect();
    // 0x53 was `CHECK`, retired: a check is tier 1's masks (doc 18 §4).
    assert_eq!(tier2, (TOKEN..=WRAP).filter(|&opcode| opcode != 0x53).collect(), "tier 2's words");
    let walker: std::collections::BTreeSet<u8> = admitted.difference(&tier2).copied().collect();
    assert_eq!(walker, OPCODES.into_iter().collect());
    assert_eq!(metal, walker, "an opcode the verifier admits that Metal cannot run, or the reverse");
}
