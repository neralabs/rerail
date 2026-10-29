//! Reproducible semantic comparisons and bounded amplification measurements.
use super::*;
use ast_grep_core::tree_sitter::LanguageExt;
use tree_sitter::StreamingIterator;

type CaptureRows = Vec<Vec<(String, u32, usize, usize)>>;

fn capture_rows(program: &Program, output: &[Output]) -> CaptureRows {
    output
        .iter()
        .map(|found| {
            found
                .captures
                .iter()
                .map(|c| {
                    (
                        program.label_name(c.label).unwrap().to_owned(),
                        c.node,
                        c.span.start,
                        c.span.end,
                    )
                })
                .collect()
        })
        .collect()
}

fn native_rows(query: &str, source: &str, tape: &TapeDoc) -> CaptureRows {
    let language = SupportLang::Python.get_ts_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let query = tree_sitter::Query::new(&language, query).unwrap();
    let mut nodes = HashMap::new();
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        let index = nodes.len() as u32;
        assert_eq!(tape.transducer_kind(index), node.kind_id());
        assert_eq!(tape.transducer_span(index), node.byte_range());
        nodes.insert(node.id(), index);
        let mut cursor = node.walk();
        let children = node.children(&mut cursor).collect::<Vec<_>>();
        pending.extend(children.into_iter().rev());
    }
    assert_eq!(nodes.len(), tape.transducer_len());
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut result = Vec::new();
    {
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(found) = matches.next() {
            result.push(
                found
                    .captures()
                    .iter()
                    .map(|c| {
                        (
                            query.capture_names()[c.index as usize].to_owned(),
                            nodes[&c.node.id()],
                            c.node.start_byte(),
                            c.node.end_byte(),
                        )
                    })
                    .collect(),
            );
        }
    }
    assert!(
        !cursor.did_exceed_match_limit(),
        "native query oracle exhausted its match limit"
    );
    result
}

fn without_ordinals(result: &Result<Vec<Output>, Error>) -> Result<Vec<Output>, Error> {
    result.clone().map(|mut rows| {
        for row in &mut rows {
            row.local_ordinal = 0;
        }
        rows
    })
}

fn first_difference(expected: &[Output], actual: &[Output]) -> Option<serde_json::Value> {
    (0..expected.len().max(actual.len())).find_map(|index| {
        let a = expected.get(index);
        let b = actual.get(index);
        (a != b).then(|| {
            serde_json::json!({
                "index": index, "early": format!("{a:?}"), "variant": format!("{b:?}"),
            })
        })
    })
}

fn compare_case(
    name: &str,
    query: Option<&str>,
    source: &str,
    program: &Program,
    roots: Option<&[u32]>,
) {
    let tape = document(source);
    let all_roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
    let roots = roots.unwrap_or(&all_roots);
    let (expected, early) = run(program, &tape, roots, Strategy::Early);
    let baseline_complete = early.stop.is_none();
    if early.stop.is_none() {
        assert_eq!(
            expected,
            program.execute_roots(&tape, roots.iter().copied()),
            "early experiment diverged from independent CPU oracle: {name}"
        );
    }
    // Native QueryCursor does not implement Tempo's host predicates or transforms.
    // Compare only structural patterns, and only when both run the full root set.
    let native = query
        .filter(|_| {
            program.predicates.is_empty()
                && program.capture_predicates.is_empty()
                && program.directives.is_empty()
                && roots == all_roots
        })
        .map(|q| native_rows(q, source, &tape));
    for (strategy, actual, measured) in std::iter::once((Strategy::Early, expected.clone(), early))
        .chain(
            [Strategy::Deferred, Strategy::DeferredExact]
                .into_iter()
                .map(|s| {
                    let (actual, measured) = run(program, &tape, roots, s);
                    (s, actual, measured)
                }),
        )
    {
        let native_equal = native.as_ref().and_then(|rows| {
            actual
                .as_ref()
                .ok()
                .map(|a| capture_rows(program, a) == *rows)
        });
        let native_multiset_equal = native.as_ref().and_then(|rows| {
            actual.as_ref().ok().map(|a| {
                let mut actual = capture_rows(program, a);
                let mut expected = rows.clone();
                actual.sort();
                expected.sort();
                actual == expected
            })
        });
        let native_difference = native.as_ref().and_then(|expected| {
            let actual = capture_rows(program, actual.as_ref().ok()?);
            (0..expected.len().max(actual.len())).find_map(|index| {
                (expected.get(index) != actual.get(index)).then(|| {
                    serde_json::json!({
                        "index": index, "native": expected.get(index), "variant": actual.get(index),
                    })
                })
            })
        });
        let comparable = measured.stop.is_none() && baseline_complete;
        let difference = match (&expected, &actual) {
            (Ok(a), Ok(b)) => first_difference(a, b),
            _ => None,
        };
        eprintln!(
            "ADMISSION_SPIKE {}",
            serde_json::json!({
                "case": name, "query": query, "source_sha256": format!("{:x}", Sha256::digest(source)),
                "source_bytes": source.len(), "roots": roots.len(), "strategy": format!("{strategy:?}"),
                "measurement": measured, "complete": measured.stop.is_none(),
                "outputs": actual.as_ref().ok().map(Vec::len), "error": actual.as_ref().err().map(|e| format!("{e:?}")),
                "full_cpu_equal": comparable.then_some(actual == expected),
                "cpu_equal_without_ordinals": comparable.then_some(without_ordinals(&actual) == without_ordinals(&expected)),
                "native_ordered_captures_equal": native_equal, "native_capture_multiset_equal": native_multiset_equal,
                "native_outputs": native.as_ref().map(Vec::len), "native_first_difference": native_difference,
                "first_difference": difference,
            })
        );
    }
}

#[test]
fn native_oracle_preserves_capture_identity_geometry_and_order() {
    let source = "# a\n# b\n";
    let query = "(module (comment) @child)";
    let tape = document(source);
    let expected = vec![
        vec![("child".to_owned(), 1, 0, 3)],
        vec![("child".to_owned(), 2, 4, 7)],
    ];
    assert_eq!(native_rows(query, source, &tape), expected);
    let program =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    assert_eq!(
        capture_rows(&program, &program.execute(&tape).unwrap()),
        expected
    );
}

#[test]
fn corrected_wildcards_capture_extra_nodes_in_every_admission_mode() {
    let source = "# a\n# b\n";
    let query = "(module (_) @child)";
    let tape = document(source);
    let program =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    assert_eq!(
        native_rows(query, source, &tape),
        vec![
            vec![("child".to_owned(), 1, 0, 3)],
            vec![("child".to_owned(), 2, 4, 7)],
        ]
    );
    let expected = native_rows(query, source, &tape);
    assert_eq!(
        capture_rows(&program, &program.execute(&tape).unwrap()),
        expected
    );
    for strategy in [Strategy::Early, Strategy::Deferred, Strategy::DeferredExact] {
        assert_eq!(
            capture_rows(
                &program,
                &run(&program, &tape, &[0, 1, 2], strategy).0.unwrap()
            ),
            expected
        );
    }
}

#[test]
fn native_query_and_corrected_cpu_retain_both_capture_alternatives() {
    let source = "# node\n";
    let query = "(module [(comment) @x @y (comment) @x]+ @z)";
    let tape = document(source);
    let program =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    let capture = |name: &str| (name.to_owned(), 1, 0, 6);
    let expected = vec![
        vec![capture("x"), capture("y"), capture("z")],
        vec![capture("x"), capture("z")],
    ];
    assert_eq!(native_rows(query, source, &tape), expected);
    let early = program.execute(&tape).unwrap();
    assert_eq!(capture_rows(&program, &early), expected);
    for strategy in [Strategy::Deferred, Strategy::DeferredExact] {
        let (actual, measured) = run(&program, &tape, &[0, 1], strategy);
        assert_eq!(measured.stop, None);
        assert_eq!(capture_rows(&program, &actual.unwrap()), expected);
    }
}

#[test]
fn structural_queries_compare_early_and_deferred_with_native_tree_sitter() {
    let queries = [
        "(module (comment)+ (comment) @middle (comment)+)",
        "(module ((comment) @item)+)",
        "(module [(comment) @left (comment) @right]+)",
        "(module (comment)? @maybe (comment)* @rest)",
        "(module [(comment) @x @y (comment) @x]+ @z)",
        "(module [(comment) @x (comment) @x @y]+ @z)",
        "(module ((comment) @x @x)+)",
        "(module (_) @child)",
    ];
    let language = SupportLang::Python.get_ts_language();
    for (query_index, query) in queries.into_iter().enumerate() {
        let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
        for count in [0, 1, 2, 3, 6, 10] {
            let source = "# node\n".repeat(count);
            compare_case(
                &format!("structural/{query_index}/{count}"),
                Some(query),
                &source,
                &program,
                None,
            );
        }
        compare_case(
            &format!("structural/{query_index}/gaps"),
            Some(query),
            "# first\npass\n# second\n# third\nvalue\n# fourth\n",
            &program,
            None,
        );
    }
    let query = "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))))";
    let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
    compare_case(
        "nested_optional_types",
        Some(query),
        "fields: ClassVar[dict[str, Field[str]]]\nfile: IO[str] | IO[bytes] | None\n",
        &program,
        None,
    );
}

// A synchronous frontier model for the actual diamond bytecode. Each iteration
// processes one instruction, then merges equal active PCs. It keeps no seen table.
// All other state is constant in this fixture. This is not a general executor.
fn bulk_diamond(program: &Program) -> (usize, usize, usize) {
    let mut frontier = vec![0_usize];
    let (mut steps, mut peak, mut emitted) = (0, 1, 0);
    while !frontier.is_empty() {
        let mut successors = Vec::new();
        for pc in frontier {
            steps += 1;
            let (opcode, _, operand) = split(program.words[pc]);
            match opcode {
                REPEAT_BEGIN => successors.push(pc + 1),
                FORK => successors.extend([pc + 1, usize::from(operand)]),
                GOTO => successors.push(usize::from(operand)),
                EMIT => emitted += 1,
                _ => panic!("fixture model received an unsupported opcode"),
            }
        }
        peak = peak.max(successors.len());
        successors.sort_unstable();
        successors.dedup();
        frontier = successors;
    }
    (emitted, peak, steps)
}

#[test]
fn bounded_bulk_frontiers_prevent_diamond_amplification_without_history() {
    for depth in [1, 4, 8, 12, 16] {
        let program = diamond(depth);
        let (emitted, peak, steps) = bulk_diamond(&program);
        assert_eq!((emitted, peak, steps), (1, 2, 3 * depth + 2));
        eprintln!(
            "ADMISSION_BULK_DIAMOND {}",
            serde_json::json!({"depth": depth, "emitted": emitted, "peak_successor_states": peak, "instruction_visits": steps})
        );
        compare_case(
            &format!("diamond/{depth}"),
            None,
            "name\n",
            &program,
            Some(&[0]),
        );
    }
}

#[test]
fn unequal_branch_depths_need_final_deduplication_after_active_bulk_reduction() {
    for (depth, visits, peak) in [
        (1, 7, 2),
        (4, 46, 7),
        (8, 154, 14),
        (12, 326, 20),
        (16, 562, 27),
    ] {
        let mut words = vec![word(REPEAT_BEGIN, 0, 0)];
        for _ in 0..depth {
            let pc = words.len() as u16;
            words.extend([
                word(FORK, 0, pc + 2),
                word(GOTO, 0, pc + 4),
                word(GOTO, 0, pc + 3),
                word(GOTO, 0, pc + 4),
            ]);
        }
        words.push(word(EMIT, 0, 0));
        let program = repeated(words);
        let (emitted, actual_peak, actual_visits) = bulk_diamond(&program);
        assert_eq!(
            (emitted, actual_peak, actual_visits),
            (depth + 1, peak, visits)
        );
        assert_eq!(
            program
                .execute_roots(&document("name\n"), [0])
                .unwrap()
                .len(),
            1
        );
        eprintln!(
            "ADMISSION_BULK_UNEQUAL {}",
            serde_json::json!({"depth": depth, "emitted": emitted, "peak_successor_states": peak, "instruction_visits": visits})
        );
        compare_case(
            &format!("unequal_diamond/{depth}"),
            None,
            "name\n",
            &program,
            Some(&[0]),
        );
    }
}

#[test]
#[ignore = "decision experiment requires the pinned query catalog and source checkout"]
fn pinned_catalog_sample_compares_every_pattern_without_omitting_failures() {
    let query_root = std::path::PathBuf::from(std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR").unwrap());
    let source_root = std::path::PathBuf::from(std::env::var("TEMPO_ALL_SCM_REPOSITORY").unwrap());
    let mut paths = std::fs::read_dir(query_root)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "scm"))
        .collect::<Vec<_>>();
    paths.sort();
    let sources = [
        (
            "synthetic.py".to_owned(),
            "import re\npattern = re.compile(r'word')\ndef test_one():\n    return 2\n".to_owned(),
        ),
        (
            "Lib/test/test_doctest/sample_doctest_errors.py".to_owned(),
            std::fs::read_to_string(
                source_root.join("Lib/test/test_doctest/sample_doctest_errors.py"),
            )
            .unwrap(),
        ),
        (
            "Lib/_markupbase.py".to_owned(),
            std::fs::read_to_string(source_root.join("Lib/_markupbase.py")).unwrap(),
        ),
    ];
    let language = SupportLang::Python.get_ts_language();
    let mut index = 0;
    for path in paths {
        let content = std::fs::read_to_string(&path).unwrap();
        for query in Program::tree_sitter_top_level_patterns(&content).unwrap() {
            let program = Program::compile_tree_sitter_query_pattern(&query, &language).unwrap();
            for (name, source) in &sources {
                compare_case(
                    &format!("catalog/{index}/{name}"),
                    Some(&query),
                    source,
                    &program,
                    None,
                );
            }
            index += 1;
        }
    }
    assert_eq!(index, 335, "the entire pinned catalog must be represented");
}

// These production regressions use native Tree-sitter as an independent oracle.
// The previous experiment intentionally recorded the incorrect early result.
fn continuation_capture_contract(jit: bool) {
    let source = "# node\n";
    let tape = document(source);
    for (query, labels) in [
        (
            "(module [(comment) @x @y (comment) @x]+ @z)",
            vec![vec!["x", "y", "z"], vec!["x", "z"]],
        ),
        (
            "(module [(comment) @x @y (comment) @y @x]+ @z)",
            vec![vec!["x", "y", "z"], vec!["y", "x", "z"]],
        ),
        (
            "(module [(comment) @x @x (comment) @x]+ @z)",
            vec![vec!["x", "x", "z"], vec!["x", "z"]],
        ),
    ] {
        let program = Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let expected = labels
            .into_iter()
            .map(|labels| {
                labels
                    .into_iter()
                    .map(|label| (label.to_owned(), 1, 0, 6))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            native_rows(query, source, &tape),
            expected,
            "native query={query}"
        );
        let outputs = if jit {
            super::super::native_cpu::execute_batch(
                std::slice::from_ref(&program),
                std::slice::from_ref(&tape),
                &[vec![vec![0]]],
            )
            .unwrap()
            .remove(0)
            .remove(0)
        } else {
            program.execute_roots(&tape, [0]).unwrap()
        };
        assert_eq!(
            capture_rows(&program, &outputs),
            expected,
            "jit={jit}, query={query}"
        );
        assert_eq!(
            outputs
                .iter()
                .map(|output| (output.task_ordinal, output.local_ordinal, output.start))
                .collect::<Vec<_>>(),
            vec![(0, 0, 0), (0, 1, 0)]
        );
        assert!(outputs.iter().all(|output| {
            output.properties.is_empty()
                && output.dynamic_properties.is_empty()
                && output
                    .captures
                    .iter()
                    .all(|capture| capture.text.is_none() && capture.properties.is_empty())
        }));
    }
}

#[test]
fn interpreter_preserves_complete_capture_prefixes_and_candidate_ordinals() {
    continuation_capture_contract(false);
}

#[test]
fn cpu_jit_preserves_complete_capture_prefixes_and_candidate_ordinals() {
    continuation_capture_contract(true);
}

#[test]
fn repeated_capture_runs_stop_at_incompatible_named_siblings() {
    let source = "# first\npass\n# second\n# third\n";
    let query = "(module ((comment) @item)+)";
    let tape = document(source);
    let program =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    let expected = native_rows(query, source, &tape);
    assert_eq!(expected.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(
        capture_rows(&program, &program.execute_roots(&tape, [0]).unwrap()),
        expected
    );
}

#[test]
fn exact_admission_bounds_ordinary_captured_repetition() {
    let source = "# node\n".repeat(16);
    let query = "(module ((comment) @item)+)";
    let tape = document(&source);
    let program =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    let expected = native_rows(query, &source, &tape);
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].len(), 16);
    let (actual, measured) = run(&program, &tape, &[0], Strategy::Early);
    assert_eq!(
        measured.stop,
        None,
        "{}",
        serde_json::to_string(&measured).unwrap()
    );
    assert_eq!(capture_rows(&program, &actual.unwrap()), expected);
    eprintln!(
        "ADMISSION_CORRECTION_EVIDENCE {}",
        serde_json::to_string(&measured).unwrap()
    );
    // A single contiguous run must not enumerate all captured subsequences.
    assert!(measured.steps < 10_000, "steps={}", measured.steps);
}

#[test]
fn wildcard_and_missing_flags_preserve_native_node_meaning() {
    for (query, source) in [
        ("(module (_) @child)", "# first\n# second\n"),
        ("(_) @node", "def f()\n    pass\n"),
        ("(MISSING) @missing", "def f(:\n pass\n"),
        (r#"(MISSING ")") @missing"#, "def f(:\n pass\n"),
        ("(MISSING identifier) @missing", "if:\n pass\n"),
        ("(ERROR) @error", "def (\n"),
    ] {
        let tape = document(source);
        let program = Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let expected = native_rows(query, source, &tape);
        assert!(!expected.is_empty(), "native control must exercise {query}");
        assert_eq!(
            capture_rows(&program, &program.execute(&tape).unwrap()),
            expected,
            "{query}"
        );
        let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
        let anchors = [vec![roots]];
        let programs = std::slice::from_ref(&program);
        let tapes = std::slice::from_ref(&tape);
        let typed = super::super::execute_batch(programs, tapes, Some(&anchors)).unwrap();
        assert_eq!(
            super::super::native_cpu::execute_batch(programs, tapes, &anchors).unwrap(),
            typed,
            "JIT: {query}"
        );
        #[cfg(target_os = "macos")]
        if super::super::execution_shape::register_paths(&program.words).is_some() {
            assert_eq!(
                super::super::execute_with_backend(
                    ExecutionBackend::Metal,
                    programs,
                    tapes,
                    &anchors
                )
                .unwrap(),
                typed,
                "Metal: {query}"
            );
        }
    }
}

#[test]
fn native_repetition_preserves_anonymous_sibling_boundaries_and_capture_order() {
    for (query, source) in [
        ("(list ((identifier) @item)+)", "items = [a, b, c]\n"),
        (
            r#"(list [(identifier) @name "," @comma]+)"#,
            "items = [a, b, c]\n",
        ),
        (
            "(module [(comment) @x @y (comment) @y @x]+ @z)",
            "# first\n# second\n",
        ),
        (
            "(module [(comment) @item ((comment) (comment) @item)]+)",
            "# a\n# b\n# c\n",
        ),
    ] {
        let tape = document(source);
        let program = Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let expected = native_rows(query, source, &tape);
        assert!(!expected.is_empty(), "native control: {query}");
        let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
        let actual = program.execute_roots(&tape, roots.iter().copied()).unwrap();
        assert_eq!(
            capture_rows(&program, &actual),
            expected,
            "interpreter: {query}"
        );
        let actual = super::super::native_cpu::execute_batch(
            std::slice::from_ref(&program),
            std::slice::from_ref(&tape),
            &[vec![roots]],
        )
        .unwrap();
        assert_eq!(
            capture_rows(&program, &actual[0][0]),
            expected,
            "JIT: {query}"
        );
    }
}

#[test]
fn missing_patterns_reject_nested_or_multiple_node_kinds() {
    let language = SupportLang::Python.get_ts_language();
    for query in [
        "(MISSING (identifier))",
        "(MISSING identifier identifier)",
        r#"(MISSING ")"+)"#,
    ] {
        assert!(
            tree_sitter::Query::new(&language, query).is_err(),
            "native: {query}"
        );
        assert!(
            Program::compile_tree_sitter_query_pattern(query, &language).is_err(),
            "compiler: {query}"
        );
    }
}
