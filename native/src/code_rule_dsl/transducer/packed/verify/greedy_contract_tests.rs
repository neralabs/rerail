//! The query compiler's supported greedy repetition boundary, checked at construction.
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_language::LanguageExt;
use tree_sitter::StreamingIterator;

fn program(words: Vec<u32>) -> Program {
    Program::new(words, vec![b"value".to_vec()], vec![], vec![0], vec![]).unwrap()
}

fn repetition() -> Vec<u32> {
    vec![
        word(SAVE_CURSOR, 0, 0),
        word(REPEAT_BEGIN, 0, 0),
        word(CHILD, NAMED, 0),
        word(FLAGS, NAMED, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, 10),
        word(NEXT, 0, 0),
        word(FLAGS, NAMED, 0),
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 5),
        word(REPEAT_END, 0, 0),
        word(RESTORE_CURSOR, 0, 0),
        word(EMIT, 0, 0),
    ]
}

#[test]
fn greedy_contract_rejects_observable_and_reentered_query_continuations() {
    let mut captures_in_body = repetition();
    captures_in_body[7] = word(CAPTURE, 0, 0);
    let mut captures_after_exit = repetition();
    captures_after_exit[12] = word(CAPTURE, 0, 0);
    captures_after_exit.push(word(EMIT, 0, 0));
    let mut wrong_repeat = repetition();
    wrong_repeat[10] = word(REPEAT_END, 0, 1);
    // A later ordinary alternative can return to a retired greedy loop header.
    let mut reentered = repetition();
    reentered[0] = word(FORK, 0, 13);
    reentered[11] = word(STOP, 0, 0);
    reentered.extend([word(CHILD, NAMED, 0), word(GOTO, 0, 5)]);
    let cases = [
        (captures_in_body, 5),
        (captures_after_exit, 5),
        (wrong_repeat, 5),
        (reentered, 5),
        (
            vec![word(FORK, GREEDY, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)],
            0,
        ),
    ];
    for (words, pc) in cases {
        assert!(
            matches!(program(words.clone()).with_tree_sitter_query_semantics(), Err(Error::InvalidGreedyRegion(at)) if at == pc),
            "query construction must reject unsupported greedy behavior at {pc}: {words:?}"
        );
    }
    // Tempo's `admission_spike::advancing_greedy_reconvergence` program (the spike is not copied).
    let counterexample = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(SAVE_CURSOR, 0, 0),
        word(FORK, 0, 8),
        word(GOTO, 0, 4),
        word(FORK, GREEDY, 11),
        word(KIND, 0, 1),
        word(DESCEND, NAMED, 0),
        word(GOTO, 0, 4),
        word(DESCEND, NAMED, 0),
        word(FIRST, NAMED, 0),
        word(GOTO, 0, 4),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ]);
    assert!(matches!(
        counterexample.with_tree_sitter_query_semantics(),
        Err(Error::InvalidGreedyRegion(4))
    ));
}

#[test]
fn greedy_contract_keeps_nonquery_branch_multiplicity() {
    let source = "# a\n";
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let tree = StrDoc::new(source, GrammarDialect::Python);
    let image = TapeImage::encode(tree.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
    let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    let program = program(vec![
        word(FORK, GREEDY, 2),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let expected = (0..2)
        .map(|local_ordinal| Output {
            task_ordinal: 0,
            local_ordinal,
            start: 0,
            label: 0,
            captures: vec![],
            properties: vec![],
            dynamic_properties: vec![],
        })
        .collect::<Vec<_>>();
    assert_eq!(program.execute_roots(&tape, [0]), Ok(expected));
}

#[test]
fn greedy_contract_preserves_compiled_query_captures_against_native() {
    let language = GrammarDialect::Python.get_ts_language();
    let queries = [
        "(module (expression_statement) @prefix (comment)+)",
        "(module (expression_statement)? @prefix (comment)*)",
        "(module (comment)+)",
        "(module [(comment) (expression_statement)]+)",
        // Nested bodies and captured repetitions must retain their existing behavior too.
        "(module (function_definition body: (block (expression_statement)+))*)",
        "(module (comment)+ @items)",
        "(module (expression_statement) @prefix . (comment)+ .)",
        "(module (function_definition !return_type body: (block (expression_statement)+))+)",
        "(module (expression_statement) @prefix . (comment) @tail)",
        "(module . (comment) @first)",
        "(module (comment)+ @tail .)",
    ];
    let mut greedy_queries = 0;
    for query_source in queries {
        let program = Program::compile_tree_sitter_query_pattern(query_source, &language).unwrap();
        greedy_queries += usize::from(
            program
                .words
                .iter()
                .any(|&w| matches!(split(w), (FORK, GREEDY, _))),
        );
        let mut sources = [0, 1, 2, 16]
            .map(|count| format!("x\n{}", "# item\n".repeat(count)))
            .to_vec();
        sources.extend([
            "def f():\n    x\n    y\n\ndef g():\n    z\n".to_owned(),
            "# first\nx\n# last\ny\n".to_owned(),
        ]);
        for source in sources {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).unwrap();
            let tree = parser.parse(&source, None).unwrap();
            let grammar = Grammar::new(GrammarDialect::Python).unwrap();
            let image = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false).unwrap().primary;
            let tape =
                TapeDoc::restore(Arc::new(source.clone()), grammar, &image, &|| false).unwrap();
            let query = tree_sitter::Query::new(&language, query_source).unwrap();
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            let mut expected = Vec::new();
            while let Some(found) = matches.next() {
                expected.push(
                    found
                        .captures()
                        .iter()
                        .map(|c| {
                            (
                                query.capture_names()[c.index as usize].to_owned(),
                                c.node.byte_range(),
                            )
                        })
                        .collect::<Vec<_>>(),
                );
            }
            drop(matches);
            assert!(!cursor.did_exceed_match_limit());
            let actual = program
                .execute_roots(&tape, 0..tape.transducer_len() as u32)
                .unwrap();
            let captures = actual
                .iter()
                .map(|o| {
                    o.captures
                        .iter()
                        .map(|c| {
                            (
                                program.label_name(c.label).unwrap().to_owned(),
                                c.span.clone(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                captures, expected,
                "query={query_source}, source={source:?}"
            );
        }
    }
    assert_eq!(greedy_queries, 7);
}
