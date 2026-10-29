//! Test-only decision experiment. This module is absent from production builds.
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
use std::time::Instant;

#[path = "admission_spike_cases.rs"]
mod cases;
#[path = "admission_spike_interpreter.rs"]
mod interpreter;
#[path = "admission_ordered_model.rs"]
mod ordered;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Strategy {
    Early,
    Deferred,
    DeferredExact,
}

#[derive(Default, serde::Serialize)]
struct Measurement {
    raw_candidates: usize,
    exact_candidates: usize,
    raw_captures: usize,
    candidate_payload_bytes: usize,
    steps: usize,
    peak_pending: usize,
    peak_seen_keys: usize,
    sibling_edge_visits: usize,
    stop: Option<&'static str>,
    elapsed_us: u128,
    final_comparisons: usize,
}

#[derive(Clone, Copy)]
struct Limits {
    steps: usize,
    candidates: usize,
    payload_bytes: usize,
    comparisons: usize,
}

const LIMITS: Limits = Limits {
    steps: 250_000,
    candidates: 32_768,
    payload_bytes: 16 * 1024 * 1024,
    comparisons: 250_000,
};

// Full terminal state equality, not equality of published rows. Frames use their
// values because arena IDs are local to each root. Ordered captures retain ties.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct TerminalKey {
    start: u32,
    pc: u16,
    node: u32,
    cursors: Vec<Option<u32>>,
    repeats: Vec<u16>,
    captures: Vec<(u16, u32, usize, usize)>,
}

impl TerminalKey {
    fn new(path: &Path, arena: &ExecutionArena) -> Self {
        Self {
            start: path.start,
            pc: path.pc,
            node: path.node,
            cursors: arena.cursors[path.cursors as usize].clone(),
            repeats: arena.repeats[path.active_repeats as usize].clone(),
            captures: arena.ordered_captures[path.captures as usize]
                .iter()
                .map(|c| (c.label, c.node, c.start, c.end))
                .collect(),
        }
    }
}

fn deduplicate_terminals(output: Vec<Output>, keys: &[TerminalKey]) -> Vec<Output> {
    assert_eq!(output.len(), keys.len());
    let mut order = (0..keys.len()).collect::<Vec<_>>();
    order.sort_unstable_by(|&a, &b| keys[a].cmp(&keys[b]).then(a.cmp(&b)));
    order.dedup_by(|a, b| keys[*a] == keys[*b]);
    order.sort_unstable();
    let mut keep = order.into_iter().peekable();
    let mut result = Vec::new();
    for (index, mut found) in output.into_iter().enumerate() {
        if keep.peek() == Some(&index) {
            keep.next();
            found.local_ordinal = result.len() as u32;
            result.push(found);
        }
    }
    result
}

// Exact payload for these untransformed candidate records in the existing typed
// ABI. This excludes allocation padding, indexes, frames, and interpreter heaps.
fn candidate_payload(output: &Output) -> usize {
    use super::metal_batch_abi::{FinalCaptureRecord, PropertyRecord, ResultRecord};
    std::mem::size_of::<ResultRecord>()
        + output.captures.len() * std::mem::size_of::<FinalCaptureRecord>()
        + output.properties.len() * std::mem::size_of::<PropertyRecord>()
}

fn run(
    program: &Program,
    tape: &TapeDoc,
    roots: &[u32],
    strategy: Strategy,
) -> (Result<Vec<Output>, Error>, Measurement) {
    let mut measured = Measurement::default();
    let start = Instant::now();
    let result = interpreter::execute(program, tape, roots, strategy, &mut measured, LIMITS);
    measured.elapsed_us = start.elapsed().as_micros();
    (result, measured)
}

#[test]
fn interpreter_snapshot_matches_the_current_independent_oracle() {
    let source = include_str!("../packed.rs");
    let start = source.find("    fn execute_roots_from(").unwrap();
    let end = start
        + source[start..]
            .find("\n}\n\n/// Executes several programs")
            .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&source.as_bytes()[start..end])),
        interpreter::ORACLE_SHA256,
        "regenerate and inspect the isolated experiment after any oracle change"
    );
}

#[test]
fn experiment_capacity_refusal_never_reports_complete_results() {
    let tape = document("name\n");
    let mut measured = Measurement::default();
    let result = interpreter::execute(
        &diamond(8),
        &tape,
        &[0],
        Strategy::Deferred,
        &mut measured,
        Limits {
            candidates: 4,
            ..LIMITS
        },
    );
    assert_eq!(result, Err(Error::BackendCapacity));
    assert_eq!(measured.stop, Some("experiment_candidate_limit"));
    assert_eq!(measured.raw_candidates, 4);
    assert_eq!(measured.exact_candidates, 0);
}

#[test]
fn deferred_exact_deduplication_preserves_distinct_cursor_frames() {
    let program = repeated(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 12),
        word(FORK, 0, 7),
        word(SAVE_CURSOR, 0, 0),
        word(GOTO, 0, 9),
        word(STOP, 0, 0),
        word(STOP, 0, 0),
        word(SAVE_CURSOR, 0, 1),
        word(GOTO, 0, 9),
        word(EMIT, 0, 0),
        word(STOP, 0, 0),
        word(STOP, 0, 0),
        word(CAPTURE, 0, 1),
        word(EMIT, 0, 1),
    ]);
    let tape = document("name\n");
    let expected = program.execute_roots(&tape, [0]).unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].local_ordinal, 2);
    let (actual, measured) = run(&program, &tape, &[0], Strategy::DeferredExact);
    assert_eq!(actual, Ok(expected));
    assert_eq!(measured.exact_candidates, 3);
}

#[test]
fn non_query_multiplicity_and_operand_errors_remain_observable() {
    let tape = document("name\n");
    let mut program = diamond(4);
    program.tree_sitter_query = false;
    let expected = program.execute_roots(&tape, [0]).unwrap();
    assert_eq!(expected.len(), 16);
    assert_eq!(
        expected.iter().map(|o| o.local_ordinal).collect::<Vec<_>>(),
        (0..16).collect::<Vec<_>>()
    );
    for strategy in [Strategy::Early, Strategy::Deferred, Strategy::DeferredExact] {
        assert_eq!(run(&program, &tape, &[0], strategy).0, Ok(expected.clone()));
    }
    for words in [
        vec![
            word(SAVE_CURSOR, 0, 500),
            word(SAVE_CURSOR, 0, 0),
            word(RESTORE_CURSOR, 0, 500),
            word(EMIT, 0, 0),
        ],
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 1),
            word(EMIT, 0, 0),
        ],
    ] {
        let program = repeated(words);
        let expected = program.execute_roots(&tape, [0]);
        assert!(matches!(expected, Err(Error::InvalidOperand(_))));
        for strategy in [Strategy::Early, Strategy::Deferred, Strategy::DeferredExact] {
            let (actual, measured) = run(&program, &tape, &[0], strategy);
            assert_eq!(actual, expected);
            assert_eq!(measured.stop, None);
        }
    }
}

fn document(source: &str) -> TapeDoc {
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}

fn repeated(words: Vec<u32>) -> Program {
    let mut program = Program::new(
        words,
        vec![b"x".to_vec(), b"y".to_vec(), b"z".to_vec()],
        vec![],
        vec![0, 1, 2],
        vec![],
    )
    .unwrap();
    program.tree_sitter_query = true;
    program
}

fn diamond(depth: usize) -> Program {
    let mut words = vec![word(REPEAT_BEGIN, 0, 0)];
    for _ in 0..depth {
        let pc = words.len() as u16;
        words.extend([
            word(FORK, 0, pc + 2),
            word(GOTO, 0, pc + 3),
            word(GOTO, 0, pc + 3),
        ]);
    }
    words.push(word(EMIT, 0, 0));
    repeated(words)
}

#[test]
fn deferred_generation_exposes_exact_reconvergence_amplification() {
    let tape = document("name\n");
    let program = diamond(8);
    let expected = program.execute_roots(&tape, [0]).unwrap();
    assert_eq!(expected.len(), 1);
    assert!(expected[0].captures.is_empty());
    for (strategy, raw, exact) in [
        (Strategy::Early, 1, 1),
        (Strategy::Deferred, 256, 256),
        (Strategy::DeferredExact, 256, 1),
    ] {
        let (actual, measured) = run(&program, &tape, &[0], strategy);
        assert_eq!(actual, Ok(expected.clone()), "{strategy:?}");
        assert_eq!(measured.raw_candidates, raw, "{strategy:?}");
        assert_eq!(measured.exact_candidates, exact, "{strategy:?}");
    }
}

#[test]
fn exact_admission_and_deferred_containment_preserve_complete_output() {
    let tape = document("name\n");
    let program = repeated(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 5),
        word(CAPTURE, 0, 0),
        word(CAPTURE, 0, 1),
        word(GOTO, 0, 6),
        word(CAPTURE, 0, 0),
        word(CAPTURE, 0, 2),
        word(EMIT, 0, 0),
    ]);
    let expected = program.execute_roots(&tape, [0]).unwrap();
    assert_eq!(expected.len(), 2);
    assert_eq!(
        expected[0]
            .captures
            .iter()
            .map(|c| c.label)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        run(&program, &tape, &[0], Strategy::Early).0,
        Ok(expected.clone())
    );
    let mut additional = expected[0].clone();
    additional.local_ordinal = 1;
    additional.captures.remove(1);
    assert_eq!(expected[1], additional);
    let all = expected;
    for strategy in [Strategy::Deferred, Strategy::DeferredExact] {
        assert_eq!(
            run(&program, &tape, &[0], strategy).0,
            Ok(all.clone()),
            "{strategy:?}"
        );
    }
}

#[test]
fn deferred_exact_deduplication_reconstructs_candidate_numbering() {
    let tape = document("name\n");
    let program = repeated(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 7),
        word(FORK, 0, 5),
        word(GOTO, 0, 6),
        word(STOP, 0, 0),
        word(GOTO, 0, 6),
        word(EMIT, 0, 0),
        word(CAPTURE, 0, 1),
        word(EMIT, 0, 1),
    ]);
    let expected = program.execute_roots(&tape, [0]).unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].local_ordinal, 1);
    assert_eq!(
        run(&program, &tape, &[0], Strategy::Early).0,
        Ok(expected.clone())
    );
    assert_eq!(
        run(&program, &tape, &[0], Strategy::DeferredExact).0,
        Ok(expected.clone())
    );
    let mut extra = expected;
    extra[0].local_ordinal = 2;
    assert_eq!(run(&program, &tape, &[0], Strategy::Deferred).0, Ok(extra));
}

#[test]
fn rejected_cursor_restoring_cycle_requires_more_than_active_batch_deduplication() {
    let tape = document("# first\n# second\n");
    // Keep the historical execution characterization on deliberately unchecked
    // test data. The production validation boundary must now reject this input.
    let mut program = repeated(vec![word(EMIT, 0, 0)]);
    program.words = vec![
        word(REPEAT_BEGIN, 0, 0),
        word(SAVE_CURSOR, 0, 0),
        word(FORK, 0, 6),
        word(NEXT, NAMED, 0),
        word(RESTORE_CURSOR, 0, 0),
        word(GOTO, 0, 2),
        word(EMIT, 0, 0),
    ];
    assert_eq!(program.validate(), Err(Error::NonProgressingCycle(5)));
    let expected = vec![Output {
        task_ordinal: 0,
        local_ordinal: 0,
        start: 1,
        label: 0,
        captures: vec![],
        properties: vec![],
        dynamic_properties: vec![],
    }];
    assert_eq!(program.execute_roots(&tape, [1]), Ok(expected.clone()));
    assert_eq!(run(&program, &tape, &[1], Strategy::Early).0, Ok(expected));
    for strategy in [Strategy::Deferred, Strategy::DeferredExact] {
        let mut measured = Measurement::default();
        let result = interpreter::execute(
            &program,
            &tape,
            &[1],
            strategy,
            &mut measured,
            Limits {
                steps: 64,
                ..LIMITS
            },
        );
        assert_eq!(result, Err(Error::WorkLimit));
        assert_eq!(measured.stop, Some("experiment_step_limit"));
        assert_eq!(measured.steps, 65);
        assert_eq!(measured.raw_candidates, 0);
    }

    // Independent synchronous model for this fixture's exact bytecode. Unlike
    // deferred DFS, it reaches EMIT. Exact terminal reduction still cannot make
    // the recurring active state disappear. The snapshot includes frame values.
    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct Snapshot {
        pc: usize,
        node: u32,
        cursor: Option<u32>,
        repeats: Vec<u16>,
    }
    let mut active = vec![Snapshot {
        pc: 0,
        node: 1,
        cursor: None,
        repeats: vec![],
    }];
    let mut terminal = Vec::new();
    let mut entry = None;
    let mut recurrences = 0;
    for _ in 0..34 {
        let mut next = Vec::new();
        for mut state in active {
            let (op, mode, operand) = split(program.words[state.pc]);
            match op {
                REPEAT_BEGIN => state.repeats.push(operand),
                SAVE_CURSOR => state.cursor = Some(state.node),
                FORK => {
                    let mut alternative = state.clone();
                    alternative.pc = usize::from(operand);
                    next.push(alternative);
                }
                NEXT => {
                    state.node = tape
                        .transducer_next_sibling(state.node, mode == NAMED)
                        .unwrap();
                }
                RESTORE_CURSOR => state.node = state.cursor.unwrap(),
                GOTO => {
                    state.pc = usize::from(operand);
                    next.push(state);
                    continue;
                }
                EMIT => {
                    terminal.push(state);
                    continue;
                }
                _ => panic!("unexpected opcode in the cycle fixture"),
            }
            state.pc += 1;
            next.push(state);
        }
        next.sort();
        next.dedup();
        assert!(!next.is_empty());
        if next.len() == 1 && next[0].pc == 2 {
            if let Some(first) = &entry {
                assert_eq!(&next[0], first);
                recurrences += 1;
            } else {
                entry = Some(next[0].clone());
            }
        }
        active = next;
    }
    assert_eq!(recurrences, 8);
    assert_eq!(
        active,
        vec![Snapshot {
            pc: 2,
            node: 1,
            cursor: Some(1),
            repeats: vec![0]
        }]
    );
    assert_eq!(terminal.len(), 8);
    terminal.sort();
    terminal.dedup();
    assert_eq!(
        terminal,
        vec![Snapshot {
            pc: 6,
            node: 1,
            cursor: Some(1),
            repeats: vec![0]
        }]
    );
    eprintln!(
        "ACTIVE_CYCLE_EVIDENCE {}",
        serde_json::json!({
            "rejectedByValidation": true, "uncheckedCpuOutputs": 1, "deferredInstructionLimit": 64,
            "synchronousWaves": 34, "exactStateRecurrences": recurrences,
            "rawTerminals": 8, "uniqueTerminals": terminal.len(), "remainingActive": active.len(),
        })
    );
}

/// An advancing loop with a greedy continuation and a later reconverging branch.
/// Both branches can reach the same loop state after its greedy token has retired.
pub(super) fn advancing_greedy_reconvergence() -> (Program, TapeDoc, Vec<Output>) {
    let tape = document("# a\n# b\n");
    assert_eq!(tape.transducer_children(0).collect::<Vec<_>>(), [1, 2]);
    let program = repeated(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(SAVE_CURSOR, 0, 0),
        word(FORK, 0, 8),
        word(GOTO, 0, 4),
        word(FORK, GREEDY, 11),
        word(KIND, 0, tape.transducer_kind(0)),
        word(DESCEND, NAMED, 0),
        word(GOTO, 0, 4),
        word(DESCEND, NAMED, 0),
        word(FIRST, NAMED, 0),
        word(GOTO, 0, 4),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let expected = vec![Output {
        task_ordinal: 0,
        local_ordinal: 0,
        start: 0,
        label: 0,
        captures: vec![Capture {
            label: 0,
            node: 2,
            span: 4..7,
            text: None,
            properties: vec![],
        }],
        properties: vec![],
        dynamic_properties: vec![],
    }];
    (program, tape, expected)
}

#[test]
fn advancing_greedy_reconvergence_changes_results_when_admission_is_deferred() {
    let (program, tape, expected) = advancing_greedy_reconvergence();
    // Preserve the former semantics only as unchecked research evidence. Ordinary
    // query construction now rejects this unsupported greedy region upstream.
    assert_eq!(program.validate(), Err(Error::InvalidGreedyRegion(4)));
    assert_eq!(program.execute_roots(&tape, [0]), Ok(expected.clone()));
    let mut extra = expected[0].clone();
    extra.local_ordinal = 1;
    extra.captures[0].node = 1;
    extra.captures[0].span = 0..3;
    let deferred = vec![extra, expected[0].clone()];
    for strategy in [Strategy::Early, Strategy::Deferred, Strategy::DeferredExact] {
        let (actual, measured) = run(&program, &tape, &[0], strategy);
        let wanted = if strategy == Strategy::Early {
            &expected
        } else {
            &deferred
        };
        assert_eq!(actual, Ok(wanted.clone()));
        assert_eq!(measured.stop, None);
        assert_eq!(
            measured.raw_candidates,
            if strategy == Strategy::Early { 1 } else { 2 }
        );
        assert!(measured.steps < 64);
        eprintln!(
            "ADMISSION_GREEDY_RECONVERGENCE {}",
            serde_json::json!({"strategy":format!("{strategy:?}"),"measurement":measured,"captureNodes":wanted.iter().map(|output| output.captures[0].node).collect::<Vec<_>>(),"rawOrdinals":wanted.iter().map(|output| output.local_ordinal).collect::<Vec<_>>() })
        );
    }
}
