    use super::*;

    #[test]
    fn input_child_proofs_preserve_unique_and_repeated_roots_on_every_platform() {
        use super::super::{CAPTURE, CHILD, EMIT, KIND, NAMED, word};
        use crate::code_rule_dsl::tape::{Grammar, TapeImage};
        use ast_grep_core::tree_sitter::StrDoc;
        use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
        use ast_grep_core::tree_sitter::LanguageExt;
        let source = "def f():\n    pass\nf(a, b)\n";
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let file =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let language = SupportLang::Python.get_ts_language();
        let unique = Program::compile_tree_sitter_query_pattern(
            "(function_definition body: (block) @body)",
            &language,
        )
        .unwrap();
        let repeated = Program::new(
            vec![
                word(KIND, 0, language.id_for_node_kind("argument_list", true)),
                word(CHILD, NAMED, 0),
                word(KIND, 0, language.id_for_node_kind("identifier", true)),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"name".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [unique, repeated];
        let roots = ["function_definition", "argument_list"].map(|kind| {
            let kind = language.id_for_node_kind(kind, true);
            (0..file.transducer_len() as u32)
                .filter(|&node| file.transducer_kind(node) == kind)
                .collect::<Vec<_>>()
        });
        assert!(roots.iter().all(|roots| roots.len() == 1));
        let input = MetalGridInput::build(&programs, &[file], &[roots.to_vec()]).unwrap();
        let shapes = input.program_shapes(&programs).unwrap();
        assert_ne!(shapes[0].dispatch_signature & 0x80000000, 0);
        assert_eq!(shapes[1].dispatch_signature & 0x80000000, 0);
        assert_eq!(input.grid.header.semantic_tuple_count, 2);
        for (program, root) in roots.iter().enumerate() {
            let span = input
                .grid
                .evaluation_spans
                .iter()
                .find(|span| span.program == program as u32)
                .unwrap();
            let ordinal = input.grid.selector_root(span.selector, 0).unwrap();
            assert_eq!(input.grid.roots[ordinal as usize].node, root[0]);
        }
        let copied = Arc::new((*input.packed).clone());
        assert!(matches!(
            input.children.validate(&copied),
            Err(Error::InvalidAnchor)
        ));
    }

    fn shape(signature: u32) -> GridProgramShape {
        GridProgramShape {
            signature,
            class: signature % 3,
            route: signature % EXECUTION_ROUTE_COUNT as u32,
            dispatch_signature: signature,
        }
    }

    #[test]
    fn semantic_windows_select_only_intersecting_signature_blocks() {
        let grid = MetalExecutionGrid::build(
            &[
                vec![
                    (0..600).collect(),
                    (0..600).step_by(2).collect(),
                    vec![7, 11, 19],
                ],
                vec![vec![2, 4, 8], vec![2, 4, 8], vec![]],
            ],
            &[shape(1), shape(1), shape(2)],
        )
        .unwrap();
        for first in 0..=grid.header.semantic_tuple_count {
            for length in [0, 1, 17, 256, 800] {
                let last = first
                    .saturating_add(length)
                    .min(grid.header.semantic_tuple_count);
                let selected = grid.block_indices_for_range(first..last).unwrap();
                let expected = grid
                    .blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| {
                        first < last
                            && grid.block_spans[block.block_span_start as usize
                                ..(block.block_span_start + block.block_span_count) as usize]
                                .iter()
                                .any(|span| {
                                    span.semantic_start < last
                                        && span.semantic_start + span.root_count > first
                                })
                    })
                    .map(|(index, _)| index as u32)
                    .collect::<Vec<_>>();
                assert_eq!(selected, expected, "{first}..{last}");
            }
        }
        assert!(
            grid.block_indices_for_range(std::ops::Range { start: 2, end: 1 })
                .is_err()
        );
        assert!(
            grid.block_indices_for_range(0..grid.header.semantic_tuple_count + 1)
                .is_err()
        );
    }

    #[test]
    fn grid_preserves_every_semantic_tuple_without_tuple_records() {
        let inputs = vec![
            vec![
                (0..600).collect::<Vec<_>>(),
                (0..600).step_by(2).collect::<Vec<_>>(),
                vec![7, 11, 19],
            ],
            vec![vec![2, 4, 8], vec![2, 4, 8], vec![]],
        ];
        let grid = MetalExecutionGrid::build(&inputs, &[shape(1), shape(1), shape(2)]).unwrap();
        assert_eq!(grid.header.root_count, 603);
        assert_eq!(grid.header.semantic_tuple_count, 909);
        assert_eq!(grid.header.evaluation_span_count, 5);
        assert_eq!(
            grid.selectors.len(),
            4,
            "equal selectors are interned per file"
        );
        assert!(grid.blocks.iter().all(|block| block.logical_count <= 256));
        assert_eq!(
            grid.route_ranges
                .iter()
                .map(|range| range.block_count)
                .sum::<u32>(),
            grid.header.block_count
        );
        assert!(grid.blocks.len() < grid.header.semantic_tuple_count as usize / 8);

        let mut actual = Vec::new();
        for semantic_span in &grid.semantic_span_order {
            let block_span = grid.block_spans[*semantic_span as usize];
            let evaluation = grid.evaluation_spans[block_span.evaluation_span as usize];
            for offset in 0..block_span.root_count {
                let root = grid
                    .selector_root(evaluation.selector, block_span.selector_offset + offset)
                    .unwrap();
                actual.push((evaluation.file, evaluation.program, root));
            }
        }
        let mut expected = Vec::new();
        let mut file_bases = Vec::new();
        let mut base = 0_u32;
        for programs in &inputs {
            file_bases.push(base);
            base += merge_roots(programs).len() as u32;
        }
        for (file, programs) in inputs.iter().enumerate() {
            let universe = merge_roots(programs);
            for (program, roots) in programs.iter().enumerate() {
                for root in roots {
                    let ordinal = file_bases[file] + universe.binary_search(root).unwrap() as u32;
                    expected.push((file as u32, program as u32, ordinal));
                }
            }
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn selectors_choose_bounded_random_access_encodings() {
        let range = encode_selector(&[10, 11, 12, 13]).unwrap();
        assert_eq!(range.kind, SELECTOR_RANGE);
        let runs = encode_selector(&[0, 1, 2, 3, 4, 5, 100, 101, 102, 103, 104, 105]).unwrap();
        assert_eq!(runs.kind, SELECTOR_RUNS);
        let explicit = encode_selector(&[0, 1_000, 2_000]).unwrap();
        assert_eq!(explicit.kind, SELECTOR_EXPLICIT);
        let roots = (0..5000).filter(|index| index % 3 != 1).collect::<Vec<_>>();
        let mut data = Vec::new();
        let bits = append_encoded_selector(&roots, &mut data).unwrap();
        assert_eq!(bits.kind, SELECTOR_BITS);
        assert!(data.len() * 4 < roots.len() * 4);
        for (offset, &root) in roots.iter().enumerate() {
            assert_eq!(decode_selector(bits, &data, offset as u32).unwrap(), root);
        }
        for malformed in [&data[..1], &data[..4], &[0, 0][..]] {
            assert!(decode_selector(bits, malformed, 0).is_err());
        }
    }

    #[test]
    fn execution_signatures_include_predicate_graph_and_output_shape() {
        use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
        use ast_grep_core::tree_sitter::LanguageExt;
        let language = SupportLang::Python.get_ts_language();
        let queries = [
            r#"((identifier) @name (#set! role "name"))"#,
            r#"((identifier) @name (#set! @name role "name"))"#,
            r#"((identifier) @name (#match? @name "^a"))"#,
            r#"((identifier) @name (#match? @name "^z"))"#,
            r#"((identifier) @name (#not-match? @name "^a"))"#,
        ];
        let programs = queries
            .iter()
            .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        let shapes = compile_program_shapes(&programs);
        assert_ne!(shapes[0].signature, shapes[1].signature);
        assert_eq!(shapes[2].signature, shapes[3].signature);
        assert_ne!(shapes[2].signature, shapes[4].signature);
        use super::super::value::{ByteNfa, ValueProgram};
        let left = ValueProgram::Regex(ByteNfa::compile("a|b").unwrap());
        let right = ValueProgram::Regex(ByteNfa::compile("a+").unwrap());
        assert_ne!(
            super::super::execution_shape::value_execution_signature(&left),
            super::super::execution_shape::value_execution_signature(&right)
        );
        use super::super::{
            EMIT, NEXT, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END, RESTORE_CURSOR, SAVE_CURSOR, word,
        };
        let cursors = [0, 1].map(|restore| {
            Program::new(
                vec![
                    word(SAVE_CURSOR, 0, 0),
                    word(NEXT, 1, 0),
                    word(SAVE_CURSOR, 0, 1),
                    word(RESTORE_CURSOR, 0, restore),
                    word(REPEAT_BEGIN, 0, 0),
                    word(REPEAT_BUMP, 0, 0),
                    word(REPEAT_END, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"match".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
        });
        let entries = compile_entry_shapes(&cursors);
        assert_ne!(entries[0].signature, entries[1].signature);
    }

    #[test]
    fn malformed_roots_and_selector_ranges_fail_closed() {
        assert!(matches!(
            MetalExecutionGrid::build(&[vec![vec![2, 1]]], &[shape(0)]),
            Err(Error::InvalidAnchor)
        ));
        let grid = MetalExecutionGrid::build(&[vec![vec![1, 2, 3]]], &[shape(0)]).unwrap();
        assert!(matches!(
            grid.selector_root(0, 3),
            Err(Error::InvalidAnchor)
        ));
        assert!(matches!(
            grid.selector_root(1, 0),
            Err(Error::InvalidAnchor)
        ));
    }

    #[test]
    fn one_route_keeps_exact_signatures_in_separate_threadgroups() {
        let anchors = vec![vec![vec![0], vec![1], vec![2]]];
        let shapes = vec![
            GridProgramShape {
                signature: 41,
                class: 2,
                route: 4,
                dispatch_signature: 17,
            },
            GridProgramShape {
                signature: 73,
                class: 4,
                route: 4,
                dispatch_signature: 73,
            },
            GridProgramShape {
                signature: 41,
                class: 2,
                route: 4,
                dispatch_signature: 17,
            },
        ];
        let grid = MetalExecutionGrid::build(&anchors, &shapes).unwrap();
        assert_eq!(grid.blocks.len(), 2);
        assert_eq!(grid.blocks[0].dispatch_signature, 17);
        assert_eq!(grid.blocks[0].logical_count, 2);
        assert_eq!(grid.blocks[1].dispatch_signature, 73);
        assert_eq!(grid.blocks[1].logical_count, 1);
        let ordinals = grid
            .semantic_span_order
            .iter()
            .map(|physical| grid.block_spans[*physical as usize].semantic_start)
            .collect::<Vec<_>>();
        assert_eq!(ordinals, [0, 1, 2]);
        let mut malformed = grid;
        malformed.blocks[0].dispatch_signature = 73;
        assert!(matches!(
            malformed.validate(&shapes),
            Err(Error::InvalidAnchor)
        ));
    }

    #[test]
    fn large_evaluation_splits_only_at_actual_block_boundaries() {
        let inputs = vec![vec![
            (0..3).collect::<Vec<_>>(),
            (0..1024).collect::<Vec<_>>(),
        ]];
        let grid = MetalExecutionGrid::build(&inputs, &[shape(4), shape(4)]).unwrap();
        assert_eq!(grid.header.semantic_tuple_count, 1027);
        assert_eq!(grid.blocks.len(), 5);
        assert_eq!(grid.block_spans.len(), 6);
        assert_eq!(
            grid.semantic_span_order
                .iter()
                .map(|&index| grid.block_spans[index as usize].semantic_start)
                .collect::<Vec<_>>(),
            vec![0, 3, 256, 512, 768, 1024]
        );
    }

    #[test]
    fn metadata_scales_with_spans_and_blocks_instead_of_tuples() {
        let roots = (0..100_000).collect::<Vec<_>>();
        let inputs = vec![vec![roots.clone(), roots.clone(), roots]];
        let grid = MetalExecutionGrid::build(&inputs, &[shape(4), shape(4), shape(4)]).unwrap();
        assert_eq!(grid.header.semantic_tuple_count, 300_000);
        assert_eq!(grid.selectors.len(), 1);
        assert!(grid.metadata_bytes() < 90_000);
    }
