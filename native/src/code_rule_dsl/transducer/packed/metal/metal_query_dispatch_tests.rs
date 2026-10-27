//! Complete register layouts cannot share only a structural entry signature.
use super::*;

#[test]
fn query_predicate_counts_preserve_complete_layouts_when_entries_are_equal() {
    let long = format!("# {}", "a".repeat(80));
    let programs = (0..4)
        .map(|case| {
            let text = if case < 2 { "^# a$" } else { &long };
            let predicate = if case < 2 {
                TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(text).unwrap(),
                }
            } else {
                TextPredicate::Equal(1)
            };
            let count = case % 2 + 1;
            let mut words = vec![word(CAPTURE, 0, 0)];
            words.extend((0..count).map(|index| word(ASSERT, 0, index as u16)));
            words.push(word(EMIT, 0, 0));
            Program::new_with_capture_predicates(
                words,
                vec![b"item".to_vec(), text.as_bytes().to_vec()],
                vec![predicate],
                (0..count)
                    .map(|_| CapturePredicate::Text {
                        capture: 0,
                        predicate: 0,
                        negated: false,
                        any: false,
                    })
                    .collect(),
                vec![0],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = [tape("# a"), tape(&long)];
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(expected.errors, [vec![None; 4], vec![None; 4]]);
    assert_eq!(
        expected.outputs[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1, 0, 0]
    );
    assert_eq!(
        expected.outputs[1].iter().map(Vec::len).collect::<Vec<_>>(),
        [0, 0, 1, 1]
    );
    for output in expected.outputs.iter().flatten().flatten() {
        assert_eq!(output.label, 0);
        assert_eq!(output.captures.len(), 1);
        assert_eq!(output.captures[0].label, 0);
        assert_eq!(output.captures[0].node, 0);
    }
    assert_eq!(
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap(),
        expected
    );
    for limit in [0, 1] {
        let actual = metal_program_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 4]),
        )
        .unwrap()
        .into_outcome()
        .unwrap();
        assert_eq!(actual.errors, expected.errors);
        let mut outputs = expected.outputs.clone();
        for rows in outputs.iter_mut().flatten() {
            rows.truncate(limit as usize);
        }
        assert_eq!(actual.outputs, outputs);
    }
}
