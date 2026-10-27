//! Behavioral validation of structural progress, independent of execution backend.
use super::*;
use crate::policy_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::SupportLang;

fn program(words: Vec<u32>) -> Result<Program, Error> {
    Program::new(words, vec![b"value".to_vec()], vec![], vec![0], vec![])
}

#[test]
fn cycle_progress_rejects_restored_and_bypassed_navigation() {
    let cases = [
        // A cursor saved before the loop restores its original node every time.
        (
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(FORK, 0, 5),
                word(NEXT, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(GOTO, 0, 1),
                word(EMIT, 0, 0),
            ],
            4,
        ),
        // Saving inside the loop does not make a canceled advance productive.
        (
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(NEXT, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(GOTO, 0, 0),
            ],
            3,
        ),
        // A backward FORK needs the same progress proof as a backward GOTO.
        (
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(NEXT, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(FORK, 0, 0),
                word(EMIT, 0, 0),
            ],
            3,
        ),
        // NEXT appears in the lexical range but is not executed.
        (
            vec![word(GOTO, 0, 2), word(NEXT, NAMED, 0), word(GOTO, 0, 0)],
            2,
        ),
        // One branch advances. The other branch can repeat the same node.
        (
            vec![word(FORK, 0, 2), word(NEXT, NAMED, 0), word(GOTO, 0, 0)],
            2,
        ),
        // Visiting a sibling beneath the same parent does not advance that parent.
        (
            vec![
                word(CHILD, 0, 0),
                word(NEXT, 0, 0),
                word(PARENT, 0, 0),
                word(GOTO, 0, 0),
            ],
            3,
        ),
    ];
    for (words, pc) in cases {
        assert!(
            matches!(program(words.clone()), Err(Error::NonProgressingCycle(at)) if at == pc),
            "must reject returning edge {pc}: {words:?}"
        );
    }
}

#[test]
fn cycle_progress_preserves_advancing_cursor_and_navigation_loops() {
    let source = Arc::new("f()\ng()\n".to_owned());
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let tape = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let roots = tape.transducer_children(0).collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    let bodies = [
        vec![word(NEXT, NAMED, 0)],
        vec![
            word(SAVE_CURSOR, 0, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(NEXT, NAMED, 0),
        ],
        vec![
            word(NEXT, NAMED, 0),
            word(SAVE_CURSOR, 0, u16::MAX),
            word(CHILD, 0, 0),
            word(RESTORE_CURSOR, 0, u16::MAX),
        ],
        vec![word(CHILD, 0, 0), word(PARENT, 0, 0), word(NEXT, NAMED, 0)],
        vec![
            word(CHILD, 0, 0),
            word(CHILD, 0, 0),
            word(NEXT, 0, 0),
            word(PARENT, 0, 0),
            word(PARENT, 0, 0),
            word(NEXT, NAMED, 0),
        ],
    ];
    let expected = roots
        .iter()
        .rev()
        .enumerate()
        .map(|(ordinal, &node)| Output {
            task_ordinal: 0,
            local_ordinal: ordinal as u32,
            start: roots[0],
            label: 0,
            captures: vec![Capture {
                label: 0,
                node,
                span: tape.transducer_span(node),
                text: None,
                properties: vec![],
            }],
            properties: vec![],
            dynamic_properties: vec![],
        })
        .collect::<Vec<_>>();
    for body in bodies {
        let mut words = vec![word(FORK, 0, body.len() as u16 + 2)];
        words.extend(body);
        words.extend([word(GOTO, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
        let program = program(words).unwrap();
        assert_eq!(
            program.execute_roots(&tape, [roots[0]]),
            Ok(expected.clone())
        );
    }
    // The productive path leaves the lexical interval of the edge at PC 2.
    let noncontiguous = program(vec![
        word(FORK, 0, 8),
        word(GOTO, 0, 5),
        word(GOTO, 0, 0),
        word(STOP, 0, 0),
        word(STOP, 0, 0),
        word(NEXT, NAMED, 0),
        word(GOTO, 0, 2),
        word(STOP, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ])
    .unwrap();
    assert_eq!(noncontiguous.execute_roots(&tape, [roots[0]]), Ok(expected));
}

#[test]
fn cycle_progress_preserves_nested_query_loops_with_separate_entries() {
    use ast_grep_language::LanguageExt;
    use tree_sitter::StreamingIterator;
    let source = Arc::new("f(a)\ng(b)\n".to_owned());
    let query =
        "(module (expression_statement (call arguments: (argument_list (identifier)+)))+ @stmt)";
    let language = SupportLang::Python.get_ts_language();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let native_query = tree_sitter::Query::new(&language, query).unwrap();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&native_query, native.tree.root_node(), source.as_bytes());
    let mut native_spans = Vec::new();
    while let Some(found) = matches.next() {
        native_spans.push(
            found
                .captures()
                .iter()
                .map(|c| c.node.byte_range())
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(native_spans, vec![vec![0..4, 5..9]]);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let tape = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
    let output = program.execute_roots(&tape, [0]).unwrap();
    assert_eq!(
        output
            .iter()
            .map(|found| found
                .captures
                .iter()
                .map(|c| c.span.clone())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        native_spans
    );
    assert!(output.iter().all(|found| found.start == 0
        && found.properties.is_empty()
        && found.dynamic_properties.is_empty()));
}

#[test]
#[ignore = "requires the pinned 335-pattern query catalog"]
fn cycle_progress_compiles_every_pinned_query() {
    use ast_grep_language::LanguageExt;
    let root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR").unwrap();
    let mut paths = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths.len(), 15);
    let language = SupportLang::Python.get_ts_language();
    let mut count = 0;
    for path in paths {
        let source = std::fs::read_to_string(&path).unwrap();
        for query in Program::tree_sitter_top_level_patterns(&source).unwrap() {
            let compiled = Program::compile_tree_sitter_query_pattern(&query, &language);
            assert!(
                compiled.is_ok(),
                "{}: {query}: {:?}",
                path.display(),
                compiled.err()
            );
            count += 1;
        }
    }
    assert_eq!(count, 335);
}

#[test]
fn cycle_progress_acceptance_implies_actual_progress_for_short_sequences() {
    let source = Arc::new("f(a, b)\ng(c)\n".to_owned());
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let tape = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let nodes = tape.transducer_len() as u32;
    let operations = [
        NEXT,
        FOLLOWING,
        CHILD,
        DESCEND,
        PARENT,
        SAVE_CURSOR,
        RESTORE_CURSOR,
    ];
    let mut accepted = 0;
    let mut checked_paths = 0;
    for length in 1..=4 {
        for mut encoded in 0..operations.len().pow(length) {
            let mut body = Vec::new();
            for _ in 0..length {
                body.push(operations[encoded % operations.len()]);
                encoded /= operations.len();
            }
            let mut words = vec![word(FORK, 0, length as u16 + 2)];
            words.extend(body.iter().map(|&op| word(op, 0, 0)));
            words.extend([word(GOTO, 0, 0), word(EMIT, 0, 0)]);
            match program(words) {
                Err(Error::NonProgressingCycle(_)) => continue,
                Err(error) => panic!("unexpected rejection for {body:?}: {error:?}"),
                Ok(_) => accepted += 1,
            }
            for start in 0..nodes {
                for saved in std::iter::once(None).chain((0..nodes).map(Some)) {
                    // Evaluate one finite body directly on the tape. No admission,
                    // abstract position facts, or historical index participates.
                    let mut paths = vec![(start, saved)];
                    for &op in &body {
                        let mut next = Vec::new();
                        for (node, cursor) in paths {
                            match op {
                                SAVE_CURSOR => next.push((node, Some(node))),
                                RESTORE_CURSOR => next.extend(cursor.map(|saved| (saved, cursor))),
                                NEXT => next.extend(
                                    tape.transducer_next_sibling(node, false)
                                        .map(|node| (node, cursor)),
                                ),
                                PARENT => next.extend(
                                    tape.transducer_parent(node).map(|node| (node, cursor)),
                                ),
                                CHILD => next.extend(
                                    tape.transducer_children(node).map(|node| (node, cursor)),
                                ),
                                DESCEND => next.extend(
                                    tape.transducer_descendants(node).map(|node| (node, cursor)),
                                ),
                                FOLLOWING => {
                                    let mut following = tape.transducer_next_sibling(node, false);
                                    while let Some(node) = following {
                                        next.push((node, cursor));
                                        following = tape.transducer_next_sibling(node, false);
                                    }
                                }
                                _ => unreachable!("fixture operation"),
                            }
                        }
                        paths = next;
                    }
                    for (node, _) in paths {
                        assert!(
                            node > start,
                            "accepted {body:?} returns from node {start} to {node}, initial cursor={saved:?}"
                        );
                        checked_paths += 1;
                    }
                }
            }
        }
    }
    // These counts describe coverage, while every individual accepted path above
    // must satisfy the semantic progress invariant.
    eprintln!(
        "CYCLE_PROGRESS_COVERAGE {}",
        serde_json::json!({"bodies": (1..=4).map(|length| operations.len().pow(length)).sum::<usize>(), "accepted": accepted, "successfulPathsChecked": checked_paths, "nodes": nodes})
    );
    assert!(accepted > 0 && checked_paths > 0);
}
