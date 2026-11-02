use super::super::*;
use super::{assemble, assemble_all, disassemble};

/// The program back from its dump: the same digest (every component), the same dump.
fn round_trip(program: &Program) -> String {
    let text = disassemble(program);
    let back = assemble(&text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    assert_eq!(back.plain_digest(), program.plain_digest(), "{text}");
    assert_eq!(disassemble(&back), text);
    text
}

/// Every table: strings with bytes to escape, labels, longs, the four text predicates, every
/// capture assertion, directives, set mode.
fn tables() -> Program {
    let strings = vec![
        b"match".to_vec(),
        b"x".to_vec(),
        b"^ex\"ec\\d(ute)?$".to_vec(),
        b"(%a+)".to_vec(),
        b"key".to_vec(),
        vec![0, 0xff, b'\n'],
        b"value".to_vec(),
    ];
    let predicates = vec![
        TextPredicate::Equal(1),
        TextPredicate::Prefix(1),
        TextPredicate::Regex { source: 2, compiled: Regex::new("^ex\"ec\\d(ute)?$").unwrap() },
        TextPredicate::LuaPattern { source: 3, compiled: LuaPattern::parse(b"(%a+)").unwrap() },
    ];
    let asserts = vec![
        CapturePredicate::Equal { left: 0, right: 1, negated: true, any: false },
        CapturePredicate::Text { capture: 0, predicate: 2, negated: false, any: true },
        CapturePredicate::AnyOf { capture: 1, strings: vec![1, 5], negated: false, any: false },
        CapturePredicate::Kind { capture: 0, kind: 7, negated: true, any: true },
        CapturePredicate::Kinds { capture: 1, kinds: vec![3, 9], negated: false },
        CapturePredicate::Contains { capture: 0, strings: vec![], negated: false, any: false },
        CapturePredicate::Relation { capture: 1, kinds: vec![4], ancestor: true, negated: false },
        CapturePredicate::SameLine { left: 0, right: 1, negated: true },
    ];
    let words = vec![
        word(KIND, 0, 12),
        word(FORK, 0, 6),
        word(TEXT, 0, 2),
        word(CHILD, NAMED, 0),
        word(CAPTURE, 0, 1),
        word(SIZE, 0x80, 0),
        word(ASSERT, 0, 7),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ];
    Program::new_with_capture_predicates(words, strings, predicates, asserts, vec![0, 1], vec![100_000, u64::MAX])
        .unwrap()
        .with_directives(vec![
            Directive::Set { key: 4, value: Some(6) },
            Directive::Set { key: 4, value: None },
            Directive::Downcase { capture: 0 },
            Directive::Offset { capture: 1, offsets: [1, -2, 3, -4] },
        ])
        .unwrap()
        .in_set_mode()
}

#[test]
fn every_program_table_round_trips() {
    let text = round_trip(&tables());
    for row in ["string 5 \"\\x00\\xff\\x0a\"", "string 2 \"^ex\\\"ec\\\\d(ute)?$\"", "predicate 3 lua s3", "assert 6 relation L1 [4] ancestor=1 negated=0", "directive 3 offset L1 1 -2 3 -4", "0003 CHILD m=1 0", "0001 FORK @0006", "0008 EMIT L0  ; match"] {
        assert!(text.contains(row), "{row}\n{text}");
    }
}

#[test]
fn an_ast_pattern_program_round_trips() {
    let mut program = Program::compile_navigation().unwrap();
    program.ast_pattern = Some(PackedPattern::Internal {
        kind: 12,
        children: vec![
            PackedPattern::Terminal { text: "(".into(), named: false, kind: 3 },
            PackedPattern::Single { name: Some("A".into()), named: true },
            PackedPattern::Ellipsis(None),
            PackedPattern::Internal { kind: 4, children: vec![PackedPattern::Ellipsis(Some("REST".into()))] },
        ],
    });
    program.digest = DigestMemo::default();
    let text = round_trip(&program);
    assert!(text.contains("pattern (internal 12 (terminal 3 any \"(\") (single \"A\" named) (ellipsis -) (internal 4 (ellipsis \"REST\")))"), "{text}");
}

fn refused(text: &str) -> String {
    match assemble(text) {
        Ok(_) => panic!("assembled:\n{text}"),
        Err(error) => error,
    }
}

/// A hand-written program (no digest) assembles through the verifier, which refuses a word out
/// of its tables; a stated digest must be the program's.
#[test]
fn the_assembler_runs_the_verifier() {
    let written = "program -\n  string 0 \"match\"\n  label 0 s0\n  0000 KIND 5\n  0001 CAPTURE L0 ; a comment\n  0002 EMIT L0\n";
    let program = assemble(written).unwrap();
    assert_eq!(program.words, vec![word(KIND, 0, 5), word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
    let verifier = refused(&written.replace("EMIT L0", "EMIT L3"));
    assert!(verifier.contains("verifier") && verifier.contains("InvalidOperand(2)"), "{verifier}");
    let stated = disassemble(&program).replace("KIND 5", "KIND 6");
    assert!(refused(&stated).contains("digest"), "an edited program keeps its old digest");
    assert!(refused(&written.replace("0001 ", "0002 ")).contains("out of order"));
    // A dump's other sections are skipped; each program block assembles.
    let two = format!("wave x\n  label 1\n{}group members=2 prefix=0\n{}", disassemble(&program), disassemble(&tables()));
    assert_eq!(assemble_all(&two).unwrap().len(), 2);
}
