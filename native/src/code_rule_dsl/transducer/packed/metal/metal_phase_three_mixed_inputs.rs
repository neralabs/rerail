//! Structural cases from retained fixtures that also require phase-four output.
//! The original mixed fixtures remain required when those operators exist.
use super::*;

fn structural_parity(programs: &[Program], files: &[TapeDoc]) {
    for program in programs {
        assert!(program.predicates.is_empty());
        assert!(program.capture_predicates.is_empty());
        assert!(program.value_programs.is_empty());
        assert!(program.capture_programs.is_empty());
        assert!(program.directives.is_empty());
    }
    let roots = files
        .iter()
        .map(|file| vec![(0..file.transducer_len() as u32).collect(); programs.len()])
        .collect::<Vec<_>>();
    assert_fixture(programs, files, &roots);
}

#[test]
fn oversized_structural_shapes_preserve_occurrences_and_long_control_paths() {
    let make =
        |words| Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
    let mut programs = [9, 130]
        .map(|count| {
            let mut words = vec![word(CAPTURE, 0, 0); count];
            words.push(word(EMIT, 0, 0));
            make(words)
        })
        .to_vec();
    let mut branches = Vec::new();
    for index in 0..65 {
        branches.extend([
            word(FORK, 0, (index + 1) * 3),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ]);
    }
    branches.extend([word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
    programs.push(make(branches));
    let mut jumps = (1..=257)
        .map(|next| word(GOTO, 0, next))
        .collect::<Vec<_>>();
    jumps.push(word(EMIT, 0, 0));
    programs.push(make(jumps));
    let files = [tape("name\n")];
    let expected =
        execute_batch(&programs, &files, Some(&[vec![vec![0]; programs.len()]])).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        vec![1, 1, 66, 1]
    );
    assert_eq!(expected[0][1][0].captures.len(), 130);
    structural_parity(&programs, &files);
}

#[test]
fn structural_alternatives_and_repeated_navigation_keep_exact_capture_context() {
    let mut programs = [
        "(_ (block) @inner) @outer",
        "(call function: (identifier) @name)",
        "(function_definition name: (identifier) @name)",
        "(call function: [(identifier) (attribute attribute: (identifier))] @name)",
        "(call function: [(identifier) @name (identifier) @duplicate])",
    ]
    .map(|pattern| {
        Program::compile_tree_sitter_query_pattern(pattern, &SupportLang::Python.get_ts_language())
            .unwrap()
    })
    .to_vec();
    programs.push(
        Program::new(
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"a".to_vec(), b"b".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap(),
    );
    structural_parity(
        &programs,
        &[
            tape("def first():\n    call(one, two)\ndef second():\n    pass\n"),
            tape("if first:\n    pass\nelse:\n    call(one)\n"),
        ],
    );
}

#[test]
fn structural_terminal_expansions_preserve_named_and_anonymous_capture_pairs() {
    let programs = [Program::compile_tree_sitter_query_pattern(
        r#"(argument_list (identifier) @argument . "," @separator)"#,
        &SupportLang::Python.get_ts_language(),
    )
    .unwrap()];
    for count in 0..=6 {
        let arguments = (0..count)
            .map(|index| format!("arg{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        structural_parity(&programs, &[tape(&format!("call({arguments})\n"))]);
    }
}
