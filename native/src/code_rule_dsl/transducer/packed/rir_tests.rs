use super::super::*;
use super::{Names, assemble, assemble_all, assemble_named, disassemble};
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::matcher::Pattern;

fn python() -> Names {
    Names::of(GrammarDialect::Python)
}

/// The program back from its dump: the same digest (every component), the same dump.
fn round_trip(program: &Program, names: Option<&Names>) -> String {
    let text = disassemble(program, names);
    let back = assemble(&text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    assert_eq!(back.plain_digest(), program.plain_digest(), "{text}");
    assert_eq!(disassemble(&back, names), text);
    text
}

fn refused(text: &str) -> String {
    match assemble(text) {
        Ok(_) => panic!("assembled:\n{text}"),
        Err(error) => error,
    }
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
    let text = round_trip(&tables(), None);
    for row in [
        "grammar=none caps=interp",
        "string 5 \"\\x00\\xff\\x0a\"",
        "string 2 \"^ex\\\"ec\\\\d(ute)?$\"",
        "predicate 3 lua s3",
        "assert 6 relation L1 [#4] ancestor=1 negated=0",
        "directive 3 offset L1 1 -2 3 -4",
        "0000 KIND #12",
        "0003 CHILD m=1 0",
        "0001 FORK @0006",
        "0008 EMIT L0  ; match",
    ] {
        assert!(text.contains(row), "{row}\n{text}");
    }
}

/// With a grammar, kinds and fields print by name (named bare, anonymous quoted), the header
/// names the grammar and its tables' digest, and the names read back to the same ids.
#[test]
fn kinds_and_fields_print_by_name() {
    let names = python();
    let catalog = GrammarDialect::Python.kinds();
    let (call, identifier, name) = (catalog.kind_id("call"), catalog.kind_id("identifier"), catalog.field_id("name").unwrap());
    let open = (0..catalog.kind_count() as u16).find(|&id| catalog.kind_name(id) == Some("(") && !catalog.kind_is_named(id)).unwrap();
    let program = Program::new_with_capture_predicates(
        vec![word(KIND, 0, call), word(FIELD, 0, name), word(ABSENT_FIELD, 0, name), word(CHILD, 0, 0), word(KIND, 0, open), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        Vec::new(),
        vec![CapturePredicate::Kinds { capture: 0, kinds: vec![identifier, open], negated: false }],
        vec![0],
        Vec::new(),
    )
    .unwrap();
    let text = round_trip(&program, Some(&names));
    for row in ["grammar=python@", "0000 KIND call", "0001 FIELD name", "0002 ABSENT_FIELD name", "0004 KIND \"(\"", "assert 0 kinds L0 [identifier \"(\"] negated=0"] {
        assert!(text.contains(row), "{row}\n{text}");
    }
    let (_, read) = assemble_named(&text).unwrap();
    assert_eq!(read.map(|names| names.header()), Some(python().header()));
    // A name the grammar does not have, a name without a grammar, a grammar whose tables changed.
    assert!(refused(&text.replace("KIND call", "KIND no_such_kind")).contains("no named kind `no_such_kind`"));
    assert!(refused(&text.replace(&python().header(), "none")).contains("without a grammar"));
    assert!(refused(&text.replace(&python().header(), "python@0000000000000000")).contains("tables changed"));
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
    let text = round_trip(&program, None);
    assert!(text.contains("pattern (internal #12 (terminal #3 any \"(\") (single \"A\" named) (ellipsis -) (internal #4 (ellipsis \"REST\")))"), "{text}");
    assert!(text.contains("caps=interp,micro,pattern"), "{text}");
}

/// A hand-written program (no digest) assembles through the verifier, which refuses a word out
/// of its tables; a stated digest and caps must be the program's.
#[test]
fn the_assembler_runs_the_verifier() {
    let written = "program -\n  string 0 \"match\"\n  label 0 s0\n  0000 KIND #5\n  0001 CAPTURE L0 ; a comment\n  0002 EMIT L0\n";
    let program = assemble(written).unwrap();
    assert_eq!(program.words, vec![word(KIND, 0, 5), word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
    let verifier = refused(&written.replace("EMIT L0", "EMIT L3"));
    assert!(verifier.contains("verifier") && verifier.contains("InvalidOperand(2)"), "{verifier}");
    let stated = disassemble(&program, None).replace("KIND #5", "KIND #6");
    assert!(refused(&stated).contains("digest"), "an edited program keeps its old digest");
    let dumped = disassemble(&program, None);
    let caps = dumped.replace(&format!("caps={}", super::caps(&program)), "caps=interp,pattern");
    assert!(refused(&caps).contains("caps"), "{caps}");
    assert!(refused(&written.replace("0001 ", "0002 ")).contains("out of order"));
    assert!(refused(&written.replace("program -", "program - ir=6")).contains("ir=6"));
    // A dump's other sections are skipped; each program block assembles.
    let two = format!("wave x\n  label 1\n{}group members=2 prefix=0\n{}", disassemble(&program, None), disassemble(&tables(), None));
    assert_eq!(assemble_all(&two).unwrap().len(), 2);
}

/// A deterministic generator (an LCG): tests reproduce.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn flip(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

/// A random program from every table and opcode, or None where the verifier refuses it.
fn generated(random: &mut Random, kinds: usize) -> Option<Program> {
    let regexes = ["^a", "b+c", "(x|y)$", "\\d{2}"];
    let luas = ["(%a+)", "(%d)"];
    let mut strings: Vec<Vec<u8>> = (0..1 + random.below(4))
        .map(|_| (0..random.below(6)).map(|_| [b'a', b'"', b'\\', 0, 0xfe, b' ', b'z'][random.below(7)]).collect())
        .collect();
    let labels = (0..1 + random.below(3))
        .map(|at| {
            strings.push(format!("label{at}").into_bytes());
            (strings.len() - 1) as u16
        })
        .collect::<Vec<_>>();
    let mut predicates = Vec::new();
    for _ in 0..random.below(4) {
        predicates.push(match random.below(4) {
            0 => TextPredicate::Equal(random.below(strings.len()) as u16),
            1 => TextPredicate::Prefix(random.below(strings.len()) as u16),
            2 => {
                let pick = regexes[random.below(4)];
                strings.push(pick.as_bytes().to_vec());
                TextPredicate::Regex { source: (strings.len() - 1) as u16, compiled: Regex::new(pick).unwrap() }
            }
            _ => {
                let pick = luas[random.below(2)];
                strings.push(pick.as_bytes().to_vec());
                TextPredicate::LuaPattern { source: (strings.len() - 1) as u16, compiled: LuaPattern::parse(pick.as_bytes()).unwrap() }
            }
        });
    }
    let label = |random: &mut Random| random.below(labels.len()) as u16;
    let kind = |random: &mut Random| random.below(kinds) as u16;
    let mut asserts = Vec::new();
    for _ in 0..random.below(3) {
        let (negated, any) = (random.flip(), random.flip());
        asserts.push(match random.below(8) {
            0 => CapturePredicate::Equal { left: label(random), right: label(random), negated, any },
            1 if !predicates.is_empty() => CapturePredicate::Text { capture: label(random), predicate: random.below(predicates.len()) as u16, negated, any },
            2 => CapturePredicate::AnyOf { capture: label(random), strings: vec![random.below(strings.len()) as u16], negated, any },
            3 => CapturePredicate::Kind { capture: label(random), kind: kind(random), negated, any },
            4 => CapturePredicate::Kinds { capture: label(random), kinds: vec![kind(random), kind(random)], negated },
            5 => CapturePredicate::Contains { capture: label(random), strings: vec![], negated, any },
            6 => CapturePredicate::Relation { capture: label(random), kinds: vec![kind(random)], ancestor: random.flip(), negated },
            _ => CapturePredicate::SameLine { left: label(random), right: label(random), negated },
        });
    }
    let longs = (0..random.below(3)).map(|_| random.next()).collect::<Vec<_>>();
    let count = 1 + random.below(12);
    let words = (0..count)
        .map(|_| {
            let (opcode, _) = super::OPCODES[random.below(super::OPCODES.len())];
            let (mode, operand) = match opcode {
                KIND | SUPERTYPE => (0, kind(random).max(1)),
                FIELD | ABSENT_FIELD => (0, 1 + random.below(20) as u16),
                FLAGS => (NAMED, 2),
                TEXT => (0, random.below(predicates.len()) as u16),
                SIZE => (0, random.below(8) as u16),
                ASSERT => (0, random.below(asserts.len()) as u16),
                FIRST | LAST | CHILD | NEXT | DESCEND | FOLLOWING => (random.below(2) as u8, 0),
                CAPTURE | EMIT => (0, label(random)),
                FORK | GOTO => (0, random.below(count) as u16),
                _ => (0, 0),
            };
            word(opcode, mode, operand)
        })
        .chain([word(EMIT, 0, 0)])
        .collect::<Vec<_>>();
    let program = Program::new_with_capture_predicates(words, strings, predicates, asserts, labels, longs).ok()?;
    let program = if random.flip() {
        program.with_directives(vec![Directive::Set { key: 0, value: None }, Directive::Downcase { capture: 0 }]).ok()?
    } else {
        program
    };
    Some(if random.flip() { program.in_set_mode() } else { program })
}

/// Property (spec-tooling §2.4.3): every generated program the verifier accepts round trips,
/// with and without the grammar's names.
#[test]
fn generated_programs_round_trip() {
    let names = python();
    let kinds = GrammarDialect::Python.kinds().kind_count();
    let mut random = Random(17);
    let mut accepted = 0;
    for _ in 0..3000 {
        let Some(program) = generated(&mut random, kinds) else { continue };
        round_trip(&program, None);
        round_trip(&program, Some(&names));
        accepted += 1;
    }
    assert!(accepted > 500, "the generator reaches the verifier's accepted programs: {accepted}");
}

/// Fuzzing the assembler (spec-tooling §2.4.3), deterministically: dumps mutated byte by byte and
/// line by line never panic the assembler, and whatever it accepts is a verified program whose
/// own dump assembles to it again.
#[test]
fn the_assembler_survives_mutated_dumps() {
    let names = python();
    let kinds = GrammarDialect::Python.kinds().kind_count();
    let mut random = Random(29);
    let mut dumps = vec![disassemble(&tables(), None)];
    while dumps.len() < 40 {
        if let Some(program) = generated(&mut random, kinds) {
            dumps.push(disassemble(&program, Some(&names)));
        }
    }
    // Hand-written programs (no digest) let mutations reach the verifier.
    let unstated = dumps.iter().map(|dump| {
        let (_, rest) = dump.split_once('\n').unwrap();
        format!("program - ir=7 {}\n{rest}", dump.lines().next().unwrap().split(' ').find(|field| field.starts_with("grammar=")).unwrap())
    });
    let dumps = dumps.clone().into_iter().chain(unstated).collect::<Vec<_>>();
    let pieces = ["#", "@", "L", "[", "]", "(", ")", "\"", "\\x", "\\", "m=", "=", " ", "\n", "9", "65535", "4294967296", "-", "s0", "é", ";"];
    let (mut accepted, mut refused) = (0, 0);
    for round in 0..8000 {
        let mut text = dumps[round % dumps.len()].clone().into_bytes();
        for _ in 0..1 + random.below(3) {
            let at = random.below(text.len() + 1);
            match random.below(4) {
                0 if at < text.len() => {
                    text.remove(at);
                }
                1 => {
                    let piece = pieces[random.below(pieces.len())].as_bytes();
                    text.splice(at..at, piece.iter().copied());
                }
                2 if at < text.len() => text[at] = b"0123456789#@L[]()\" -="[random.below(21)],
                _ => {
                    let mut lines = String::from_utf8_lossy(&text).lines().map(str::to_owned).collect::<Vec<_>>();
                    let (a, b) = (random.below(lines.len()), random.below(lines.len()));
                    if a < lines.len() && b < lines.len() {
                        lines.swap(a, b);
                    }
                    text = (lines.join("\n") + "\n").into_bytes();
                }
            }
        }
        let text = String::from_utf8_lossy(&text);
        match assemble_named(&text) {
            Ok((program, grammar)) => {
                let again = disassemble(&program, grammar.as_ref());
                assert_eq!(assemble(&again).map(|back| back.plain_digest().to_owned()), Ok(program.plain_digest().to_owned()), "{again}");
                accepted += 1;
            }
            Err(_) => refused += 1,
        }
    }
    assert!(accepted > 0 && refused > 0, "{accepted} accepted, {refused} refused");
}

/// The frontends' programs (spec-tooling §2.4.2, a golden per frontend feature): ast-grep
/// patterns and literals, regexes, tree-sitter queries with predicates, quantifiers, alternation,
/// fields and directives, and the navigation descriptor; each round trips with Python's names,
/// and the dumps are `rir_goldens.rir` (`RERAIL_BLESS=1` rewrites it, deliberately). The
/// composite matcher's (the wave's) golden is `waves/matches/wave.rir`.
#[test]
fn the_frontends_dumps_are_the_goldens() {
    let names = python();
    let language = GrammarDialect::Python.kinds().grammar().expect("Python has a tree-sitter grammar").clone();
    let pattern = |source: &str| Pattern::try_new(source, GrammarDialect::Python).unwrap();
    let mut programs: Vec<(&str, Program)> = vec![
        ("ast-grep pattern", Program::compile_ast_grep_pattern(&pattern("exec($A)")).unwrap()),
        ("ast-grep pattern with a multi-capture", Program::compile_ast_grep_pattern(&pattern("def $F($$$ARGS): $$$BODY")).unwrap()),
        ("ast-grep literal", Program::compile_ast_grep_literal(&pattern("execute")).unwrap()),
        ("regex", Program::compile_regex(&Regex::new("^eval|exec$").unwrap()).unwrap()),
        ("navigation", Program::compile_navigation().unwrap()),
    ];
    for (feature, query) in [
        ("query: fields", "(function_definition name: (identifier) @name) @definition"),
        ("query: #eq?", "((identifier) @name (#eq? @name \"exec\"))"),
        ("query: #match? over alternation", "([(integer) (float)] @number (#match? @number \"^[12]\"))"),
        ("query: quantifier", "((module ((comment) @item)+) (#eq? @item \"# 0\"))"),
        ("query: directives", "((identifier) @name (#set! \"role\" \"definition\") (#downcase! @name))"),
        ("query: offset", "((identifier) @name (#offset! @name 0 1 0 -1) (#set! role \"trimmed\"))"),
    ] {
        programs.push((feature, Program::compile_tree_sitter_query_pattern(query, &language).unwrap_or_else(|error| panic!("{feature}: {error:?}"))));
    }
    programs.push(("set mode", Program::compile_ast_grep_literal(&pattern("execute")).unwrap().in_set_mode()));
    let mut dump = String::new();
    for (feature, program) in &programs {
        dump.push_str(&format!("; {feature}\n"));
        dump.push_str(&round_trip(program, Some(&names)));
    }
    assert_eq!(assemble_all(&dump).unwrap().len(), programs.len());
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/transducer/packed/rir_goldens.rir");
    if std::env::var_os("RERAIL_BLESS").is_some() {
        std::fs::write(&golden, &dump).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    assert!(expected == dump, "the dumps differ from {} (RERAIL_BLESS=1 to accept them)", golden.display());
}

/// A block program's tree rows (`13-tape-on-the-gpu.md` §3.1): node rows in rank order with an
/// optional level and the extra flag, code rows; each mask must be an output and each code row's
/// node a row.
#[test]
fn block_tree_rows_round_trip() {
    let rows = "program -\n  mode set=0 query=0\n  block 0 %0 = eq 0x7b\n  block 0 %1 = eq 0x7d\n  block 0 %2 = eq 0x22\n  block 0 output %0\n  block 0 output %1\n  block 0 output %2\n  block 0 delay 2\n  block 0 node 0 kind 3 start %0 end %1 level %0 %1\n  block 0 node 1 kind 9 start %2 end %2 extra\n  block 0 code 1 field 4 when %2\n  block 0 code 0 supertype 12 when %0\n  0000 STOP 0\n";
    let program = assemble(rows).unwrap_or_else(|e| panic!("{e}"));
    let block = &program.block_programs()[0];
    assert_eq!(block.nodes.len(), 2);
    assert_eq!(block.nodes[0].level, Some((0, 1)));
    assert!(block.nodes[1].extra && block.codes[0].field && !block.codes[1].field);
    let text = round_trip(&program, None);
    assert!(text.contains("node 1 kind 9 start %2 end %2 extra"), "{text}");
    let without = assemble(&rows.replace("  block 0 code 1 field 4 when %2\n", "")).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(without.plain_digest(), program.plain_digest(), "the rows are in the digest");
    assert!(refused(&rows.replace("node 1 kind", "node 2 kind")).contains("out of order"));
    assert!(!refused(&rows.replace("  block 0 output %2\n", "")).is_empty(), "a row's mask must be an output");
    assert!(!refused(&rows.replace("code 1 field", "code 5 field")).is_empty(), "a code row's node must be a row");
    assert!(refused(&rows.replace("extra\n", "wide\n")).contains("wide"));
}
