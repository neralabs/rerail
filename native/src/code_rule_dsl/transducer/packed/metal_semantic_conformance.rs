//! Independent semantic fixtures retained through the deletion-first cutover.
//! These parity tests remain pending until their semantic stages exist.
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
use ast_grep_core::tree_sitter::LanguageExt;

fn tape(source: &str) -> TapeDoc {
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}

#[cfg(test)]
#[path = "metal_phase_four_tests.rs"]
mod phase_four;

#[path = "metal_phase_three_mixed_inputs.rs"]
mod phase_three_mixed_inputs;

// Needs Tempo's anchor plans and matcher (`anchors`, `AstMatcher::Program`), which W does not have: ported with the anchor index
#[cfg(any())]
#[test]
#[ignore = "pinned corpus differential subset against the independent CPU interpreter"]
fn pinned_corpus_optional_body_and_injection_differential() {
    let root = std::env::var("TEMPO_ALL_SCM_REPOSITORY").unwrap();
    let query_root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR").unwrap();
    let mut paths = std::fs::read_dir(query_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
        .collect::<Vec<_>>();
    paths.sort();
    let language = SupportLang::Python.get_ts_language();
    let mut catalog = Vec::new();
    for path in paths {
        for pattern in
            Program::tree_sitter_top_level_patterns(&std::fs::read_to_string(path).unwrap())
                .unwrap()
        {
            catalog.push(Program::compile_tree_sitter_query_pattern(&pattern, &language).unwrap());
        }
    }
    assert_eq!(catalog.len(), 335);
    let programs = catalog;
    let names = [
        "Lib/_markupbase.py",
        "Lib/test/test_doctest/sample_doctest_errors.py",
        "Lib/_osx_support.py",
        "Lib/idlelib/calltip.py",
        "Lib/idlelib/configdialog.py",
    ];
    let sources = names
        .iter()
        .map(|path| std::fs::read_to_string(std::path::Path::new(&root).join(path)).unwrap())
        .collect::<Vec<_>>();
    let files = sources
        .iter()
        .map(|source| tape(source))
        .collect::<Vec<_>>();
    use crate::code_rule_dsl::anchors::{AnchorPlan, FileAnchorIndex, Selection};
    use crate::code_rule_dsl::matcher::AstMatcher;
    let plans = programs
        .iter()
        .map(|program| AnchorPlan::compile(&AstMatcher::Program(Arc::new(program.clone()))))
        .collect::<Vec<_>>();
    let plan_refs = plans.iter().collect::<Vec<_>>();
    let anchors = files
        .iter()
        .zip(&sources)
        .map(|(file, source)| {
            let native = ast_grep_core::AstGrep::doc(StrDoc::new(source, SupportLang::Python));
            let index = FileAnchorIndex::build(&native, &plan_refs, usize::MAX, || false);
            plans
                .iter()
                .map(|plan| match index.select(plan) {
                    Selection::All => (0..file.transducer_len() as u32).collect(),
                    Selection::Roots(roots) => roots.iter().map(|&root| root as u32).collect(),
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let result = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
    let trace = scope.snapshot();
    drop(scope);
    if metal_dynamic_execution::ViewConformanceScope::active() {
        assert!(
            trace
                .iter()
                .any(|record| record.kernel == "expanded_counts"),
            "the forced view conformance run must execute the operator it validates"
        );
    }

    let mut commands = BTreeMap::<u64, Vec<&str>>::new();
    let mut kernels = BTreeMap::<&str, usize>::new();
    for record in &trace {
        commands
            .entry(record.command)
            .or_default()
            .push(record.kernel.as_str());
        *kernels.entry(record.kernel.as_str()).or_default() += 1;
    }
    eprintln!(
        "PINNED_REGRESSION_COMMANDS {}",
        serde_json::json!({"commands":commands,"kernels":kernels})
    );
    eprintln!(
        "PINNED_REGRESSION_EVIDENCE {}",
        serde_json::json!({
            "patterns": programs.len(), "files": files.len(), "error": format!("{:?}", result.as_ref().err()),
            "launches": trace.len(), "last": trace.last(),
            "cpuMatches": expected.iter().flatten().map(Vec::len).sum::<usize>(),
        })
    );
    let actual = result.expect(
        "every pinned optional-body/injection evaluation must complete under the original guards",
    );
    assert!(!trace.is_empty() && trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|record| record.encoded && record.submitted && record.completed)
    );
    assert!(trace.iter().all(|record| record.peak_application_buffer_bytes <= metal_backend::METAL_BUFFER_LIMIT));
    assert_eq!(
        trace
            .iter()
            .filter(|record| record.kernel == "register_roots")
            .count(),
        1
    );
    let counts = actual
        .iter()
        .flatten()
        .flatten()
        .fold([0usize; 5], |mut counts, row| {
            counts[0] += 1;
            counts[1] += row.captures.len();
            counts[2] += row.properties.len();
            counts[3] += row
                .captures
                .iter()
                .map(|capture| capture.properties.len())
                .sum::<usize>();
            counts[4] += row.dynamic_properties.len();
            counts
        });
    eprintln!(
        "PINNED_REGRESSION_TABLES {}",
        serde_json::json!({"counts":counts})
    );
    for (file, path) in names.iter().enumerate() {
        for program in 0..programs.len() {
            assert_eq!(
                actual[file][program], expected[file][program],
                "path={path}, program={program}"
            );
        }
    }
}

#[test]
fn corpus_optional_test_bodies_and_regex_injections_match_cpu() {
    let queries = [
        r#"((function_definition name: (identifier) @_name body: (block)? @test.inside) @test.around (#match? @_name "^test_"))"#,
        r#"(call function: (attribute object: (identifier) @_re) arguments: (argument_list (string (string_start) [(string_content) @injection.content (interpolation)]+ (string_end))) (#eq? @_re "re") (#set! injection.language "regex"))"#,
    ];
    let sources = [
        "def test_suite():\n    import doctest\n    return doctest.DocTestSuite()\n",
        "def test_one():\n    pass\n\ndef test_two():\n    return 2\n",
        "def g():\n    [][0]\n\ndef errors():\n    \"\"\"example\"\"\"\n\ndef test_suite():\n    import doctest\n    return doctest.DocTestSuite()\n",
        "import re\n_first_param = re.compile(r'(?<=\\()\\w*\\,?\\s*')\n",
        "import re\npattern = re.compile(r'word')\n",
        "import re\npattern = re.compile(f'a{value}b')\n",
    ];
    let language = SupportLang::Python.get_ts_language();
    let programs = queries
        .iter()
        .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
        .collect::<Vec<_>>();
    let files = sources
        .iter()
        .map(|source| tape(source))
        .collect::<Vec<_>>();
    let anchors = files
        .iter()
        .map(|file| vec![(0..file.transducer_len() as u32).collect::<Vec<_>>(); programs.len()])
        .collect::<Vec<_>>();
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert!(!expected[0][0].is_empty());
    assert!(!expected[3][1].is_empty());
    let actual =
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors).unwrap();
    for (file, source) in sources.iter().enumerate() {
        for (program, query) in queries.iter().enumerate() {
            assert_eq!(
                actual[file][program], expected[file][program],
                "source={source:?}, query={query}"
            );
        }
    }
}

// BEGIN RETAINED PUBLIC INPUT FIXTURES
// Only input construction and independent oracle comparisons remain here.
// Private scheduling, allocation, compaction, and raw-state assertions were deleted.
trait FixtureDocument {
    fn fixture_document(&self) -> TapeDoc;
}
impl FixtureDocument for TapeDoc {
    fn fixture_document(&self) -> TapeDoc {
        self.clone()
    }
}
impl FixtureDocument for crate::code_rule_dsl::tape::TapeSource {
    fn fixture_document(&self) -> TapeDoc {
        self.restore(&|| false).unwrap()
    }
}
fn assert_fixture<F: FixtureDocument>(
    programs: &[Program],
    files: &[F],
    anchors: &[Vec<Vec<u32>>],
) {
    let files = files
        .iter()
        .map(FixtureDocument::fixture_document)
        .collect::<Vec<_>>();
    let expected =
        execute_with_backend_outcomes(ExecutionBackend::CpuInterpreter, programs, &files, anchors)
            .map(|batch| (batch.outputs, batch.errors));
    let scope = metal_backend::launch::LaunchScope::enter();
    let actual = execute_with_backend_outcomes(ExecutionBackend::Metal, programs, &files, anchors)
        .map(|batch| (batch.outputs, batch.errors));
    if actual != expected {
        let trace = scope.snapshot();
        eprintln!(
            "FIXTURE_FAILURE_TRACE {}",
            serde_json::json!({
                "attempts": trace.len(),
                "kernels": trace.iter().map(|record| &record.kernel).collect::<Vec<_>>(),
            })
        );
    }
    assert_eq!(actual, expected);
}
fn assert_value_budget(program: &Program, source: &str, limit: usize, ordered: bool) {
    use value::ValueProgram;
    let (expected, cost, actual, used) = match &program.value_programs[0] {
        ValueProgram::Lua(lua) => {
            assert!(!ordered);
            let (expected, cost) = lua.test_work_count(source.as_bytes());
            let (actual, used) = lua.test_match_with_budget(source.as_bytes(), limit);
            (expected, cost, actual, used)
        }
        ValueProgram::Regex(nfa) if ordered => {
            let (expected, cost) = nfa.test_find_work_count(source.as_bytes());
            let (actual, used) = nfa.test_find_with_budget_from(source.as_bytes(), 0, limit);
            (
                Ok(expected.is_some()),
                cost,
                actual.map(|found| found.is_some()),
                used,
            )
        }
        ValueProgram::Regex(nfa) => {
            let (expected, cost) = nfa.test_work_count(source.as_bytes());
            let (actual, used) = nfa.test_match_with_budget(source.as_bytes(), limit);
            (expected, cost, actual, used)
        }
        _ => panic!("fixture requires a Lua or NFA cost oracle"),
    };
    if limit < cost {
        assert_eq!(actual, Err(Error::WorkLimit));
        assert_eq!(used, limit + 1);
    } else {
        assert_eq!(actual, expected);
        assert_eq!(used, cost);
    }
}

fn assert_strip_budget(pattern: &str, source: &str, limit: usize) {
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let regex = regex::Regex::new(pattern).unwrap();
    let expected = regex
        .find_iter(source)
        .map(|found| [found.start() as u32, found.end() as u32])
        .collect::<Vec<_>>();
    let mut cost = 0;
    let mut search = 0;
    let mut actual_matches = Vec::new();
    loop {
        let (found, used) = nfa.test_find_work_count_from(source.as_bytes(), search);
        cost += used;
        let Some([first, end]) = found else { break };
        actual_matches.push([first, end]);
        search = end as usize + usize::from(first == end);
        while search < source.len() && source.as_bytes()[search] & 192 == 128 {
            search += 1;
        }
        if search > source.len() {
            break;
        }
    }
    assert_eq!(actual_matches, expected);
    let mut used = 0;
    let mut search = 0;
    let mut matches = Vec::new();
    let actual = (|| {
        loop {
            let (result, spent) =
                nfa.test_find_with_budget_from(source.as_bytes(), search, limit - used);
            used += spent;
            let Some([first, end]) = result? else { break };
            matches.push([first, end]);
            search = end as usize + usize::from(first == end);
            while search < source.len() && source.as_bytes()[search] & 192 == 128 {
                search += 1;
            }
            if search > source.len() {
                break;
            }
        }
        Ok(matches)
    })();
    if limit < cost {
        assert_eq!(actual, Err(Error::WorkLimit));
        assert_eq!(used, limit + 1);
    } else {
        assert_eq!(actual, Ok(expected));
        assert_eq!(used, cost);
    }
}

#[test]
fn independent_fixture_budget_oracles_enforce_actual_boundaries() {
    for (pattern, source, lua) in [
        ("a+$", "aaaab", false),
        ("^(a|aa)*$", "aaaaaaaa", false),
        ("^a*$", "aaaaaaaa", false),
        ("^.-Z$", "aaaZ", true),
        ("^%b()$", "(a(b)c)", true),
        ("^(ab)%1$", "abab", true),
    ] {
        let predicate = if lua {
            TextPredicate::LuaPattern {
                source: 0,
                compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
            }
        } else {
            TextPredicate::Regex {
                source: 0,
                compiled: regex::Regex::new(pattern).unwrap(),
            }
        };
        let program = Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
            vec![predicate],
            vec![1],
            vec![],
        )
        .unwrap();
        let cost = match &program.value_programs[0] {
            value::ValueProgram::Lua(lua) => lua.test_work_count(source.as_bytes()).1,
            value::ValueProgram::Regex(nfa) => nfa.test_work_count(source.as_bytes()).1,
            _ => unreachable!(),
        };
        assert!(cost > 0);
        for limit in [0, cost - 1, cost, cost + 1] {
            assert_value_budget(&program, source, limit, false);
        }
        if let value::ValueProgram::Regex(nfa) = &program.value_programs[0] {
            let cost = nfa.test_find_work_count(source.as_bytes()).1;
            for limit in [0, cost - 1, cost, cost + 1] {
                assert_value_budget(&program, source, limit, true);
            }
        }
    }
    for (pattern, source) in [
        ("ab", "abxabxab"),
        ("^é", "éx\n"),
        ("^", "éx\n"),
        ("^$", ""),
    ] {
        let nfa = value::ByteNfa::compile(pattern).unwrap();
        let mut cost = 0;
        let mut search = 0;
        loop {
            let (found, used) = nfa.test_find_work_count_from(source.as_bytes(), search);
            cost += used;
            let Some([first, end]) = found else { break };
            search = end as usize + usize::from(first == end);
            while search < source.len() && source.as_bytes()[search] & 192 == 128 {
                search += 1;
            }
            if search > source.len() {
                break;
            }
        }
        for limit in [0, cost - 1, cost, cost + 1] {
            assert_strip_budget(pattern, source, limit);
        }
    }
}

#[test]
fn retained_greedy_body_and_exit_public_inputs() {
    let program = Program::new(
        vec![
            word(FORK, GREEDY, 3),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 1),
        ],
        vec![b"body".to_vec(), b"exit".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    assert_fixture(&[program], &[tape("value\n")], &[vec![vec![0]]]);
}

#[test]
fn retained_sparse_cursor_ids_and_mixed_layout_public_inputs() {
    let sparse = Program::new(
        vec![
            word(SAVE_CURSOR, 0, u16::MAX),
            word(NEXT, 1, 0),
            word(RESTORE_CURSOR, 0, u16::MAX),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"comment".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    assert_fixture(
        std::slice::from_ref(&sparse),
        &[tape("# one\n# two\n")],
        &[vec![vec![1]]],
    );
    let mut words = vec![word(CHILD, 1, 0)];
    for slot in 0..64 {
        words.extend([word(SAVE_CURSOR, 0, slot), word(NEXT, 1, 0)]);
    }
    for slot in (0..64).rev() {
        words.extend([word(RESTORE_CURSOR, 0, slot), word(CAPTURE, 0, 0)]);
    }
    words.push(word(EMIT, 0, 0));
    let large = Program::new(words, vec![b"comment".to_vec()], vec![], vec![0], vec![]).unwrap();
    let source = (0..70)
        .map(|index| format!("# {index}\n"))
        .collect::<String>();
    assert_fixture(
        &[large, sparse],
        &[tape(&source), tape("# one\n# two\n")],
        &[vec![vec![0], vec![1]], vec![vec![], vec![1]]],
    );
}

#[test]
fn retained_large_relation_public_inputs() {
    let source = (0..130)
        .map(|index| format!("item_{index} = (left + right)\n"))
        .collect::<String>();
    let files = [tape(&source)];
    for (query, count) in [
        ("(module (_) @child)", 130),
        ("(module (function_definition) @function)", 0),
    ] {
        let program = Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let anchors = [vec![vec![0]]];
        let expected =
            execute_batch(std::slice::from_ref(&program), &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), count);
        assert_fixture(&[program], &files, &anchors);
    }
}

#[test]
fn retained_repeat_errors_preserve_exact_public_operand_pcs() {
    let language = SupportLang::Python.get_ts_language();
    let mut program = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(FORK, 0, 6),
            word(FOLLOWING, 1, 0),
            word(KIND, 0, language.id_for_node_kind("comment", true)),
            word(REPEAT_BUMP, 0, 1),
            word(GOTO, 0, 1),
            word(REPEAT_END, 0, 1),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    program.tree_sitter_query = true;
    for (source, pc) in [("# a\n# b\n", 4), ("# only\n", 6)] {
        let files = [tape(source)];
        let anchors = [vec![vec![1]]];
        let expected =
            execute_batch(std::slice::from_ref(&program), &files, Some(&anchors)).unwrap_err();
        assert_eq!(expected, Error::InvalidOperand(pc));
        assert_fixture(std::slice::from_ref(&program), &files, &anchors);
    }
}

mod retained_grid_runtime {
    use super::*;
    #[test]
    fn inputs_bounded_page_plan_continues_after_its_descriptor_ring_is_full() {
        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"node".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [program];
        let files = [tape(&"alpha = beta\n".repeat(400))];
        let anchors = vec![vec![(0..1100).collect::<Vec<_>>()]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_sibling_closures_preserve_gaps_and_repeat_error_pcs() {
        let language = SupportLang::Python.get_ts_language();
        let programs = [
            Program::compile_tree_sitter_query_pattern(
                "(module (comment)+ (comment) @middle (comment)+)",
                &language,
            )
            .unwrap(),
            Program::compile_tree_sitter_query_pattern(
                "(module (comment)+ (identifier) @last)",
                &language,
            )
            .unwrap(),
        ];
        let files = [
            tape("# first\npass\n# second\n# third\nvalue\n# fourth\n"),
            tape("# a\n# b\n# c\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_nested_optional_type_captures_preserve_raw_candidate_ordinals() {
        let query = "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))))";
        let programs = [Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let files = [tape(
            "fields: ClassVar[dict[str, Field[str]]]\nfile: IO[str] | IO[bytes] | None\n",
        )];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        let cpu =
            crate::code_rule_dsl::transducer::packed::native_cpu::execute_batch(&programs, &files, &anchors)
                .unwrap();
        assert_eq!(cpu, expected, "CPU JIT versus interpreter");
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_greedy_tokens_preserve_local_ordinals_and_capture_order() {
        let language = SupportLang::Python.get_ts_language();
        let queries = [
            "(module (comment)+ (comment) @middle (comment)+)",
            "(module ((comment) @item)+)",
            "(module [(comment) @left (comment) @right]+)",
        ];
        let programs = queries
            .iter()
            .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        for count in 0..=6 {
            let source = (0..count)
                .map(|index| format!("# {index}\n"))
                .collect::<String>();
            let files = [tape(&source)];
            let anchors = vec![vec![vec![0]; programs.len()]];
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_compact_stage_capture_predicates_use_the_correct_capture_generation() {
        let language = SupportLang::Python.get_ts_language();
        let module = language.id_for_node_kind("module", true);
        let early = Program::new_with_capture_predicates(
            vec![
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"node".to_vec()],
            vec![],
            vec![CapturePredicate::Kind {
                capture: 0,
                kind: module,
                negated: false,
                any: false,
            }],
            vec![0],
            vec![],
        )
        .unwrap();
        let queries = [
            "((call function: (identifier) @name) (#eq? @name \"call\"))",
            "((call function: (identifier) @name) (#not-eq? @name \"call\"))",
            "((identifier) @name (#any-of? @name \"one\" \"two\"))",
            "((identifier) @name (#not-kind-eq? @name \"integer\"))",
            "((identifier) @name (#match? @name \"^call$\"))",
        ];
        let mut programs = vec![early];
        for query in queries {
            programs.push(Program::compile_tree_sitter_query_pattern(query, &language).unwrap());
        }
        let files = [tape("call(one, two)\nother(three)\n")];
        let anchors = vec![
            programs
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    if index == 0 {
                        vec![0]
                    } else {
                        (0..files[0].transducer_len() as u32).collect()
                    }
                })
                .collect(),
        ];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert!(!expected[0][0].is_empty());
        {
            {
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_constant_identifier_predicates_reject_mixed_case_and_dunder_names() {
        let language = SupportLang::Python.get_ts_language();
        let programs = [
            r#"((identifier) @constant (#match? @constant "^_*[A-Z][A-Z\\d_]*$"))"#,
            r#"((identifier) @constant (#match? @constant "^[A-Z][A-Z_]*$"))"#,
            r#"((identifier) @constant (#lua-match? @constant "^[A-Z][A-Z_0-9]*$"))"#,
        ]
        .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap());
        let files = [tape(
            "T\nKT\nIO\nCANNOT_SUBCLASS_TYPE\nBinaryIO\nTextIO\n__name__\n__init__\n__constraints__\nDefaultStrT\nTs2\nT2\n_lower\n_A\n",
        )];
        let anchors = [vec![
            (0..files[0].transducer_len() as u32)
                .collect::<Vec<_>>();
            programs.len()
        ]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_offset_comment_predicate_uses_bounded_registers_independent_of_output_shape() {
        let language = SupportLang::Python.get_ts_language();
        let programs = [Program::compile_tree_sitter_query_pattern(
            r##"((comment) @inner @outer (#offset! @inner 0 2 0 0) (#lua-match? @outer "# .*"))"##,
            &language,
        )
        .unwrap()];
        for length in [65, 1025, 100_000] {
            let files = [tape(&format!("# {}", "a".repeat(length)))];
            let anchors = [vec![
                (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
            ]];
            let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
            assert_eq!(expected[0][0].len(), 1);
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_captured_lua_search_resumes_between_attempts_with_exact_work() {
        let language = SupportLang::Python.get_ts_language();
        let cases = [
            ("# .*", format!("#{}", "a".repeat(1025))),
            ("# .*", format!("#{}# ok", "a".repeat(1025))),
            ("a+b$", format!("# {}b", "a".repeat(129))),
            ("%b()", format!("#({})", "a".repeat(129))),
        ];
        for (pattern, source) in cases {
            let query = format!(
                r##"((comment) @inner @outer (#offset! @inner 0 2 0 0) (#lua-match? @outer "{pattern}"))"##
            );
            let programs = [Program::compile_tree_sitter_query_pattern(&query, &language).unwrap()];
            let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Lua(lua) =
                &programs[0].value_programs[0]
            else {
                unreachable!()
            };
            let (_, work) = lua.test_work_count(source.as_bytes());
            assert!(
                work > 0,
                "the retained cost oracle must account for this input"
            );
            let files = [tape(&source)];
            let anchors = [vec![
                (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
            ]];
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_captured_text_arguments_collect_in_bounded_units_and_restore_outer_state() {
        use crate::code_rule_dsl::transducer::packed::{
            ASSERT, CHILD, CapturePredicate, NEXT, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END,
            RESTORE_CURSOR, SAVE_CURSOR,
        };
        let full = format!("# {}", "a".repeat(129));
        let mut programs = Vec::new();
        for empty in [false, true] {
            for value in 0..4 {
                for negated in [false, true] {
                    for any in [false, true] {
                        let pattern = if value == 0 {
                            full.clone()
                        } else if value == 1 {
                            "# a".to_owned()
                        } else {
                            "^# a".to_owned()
                        };
                        let predicate = match value {
                            0 => TextPredicate::Equal(0),
                            1 => TextPredicate::Prefix(0),
                            2 => TextPredicate::Regex {
                                source: 0,
                                compiled: regex::Regex::new(&pattern).unwrap(),
                            },
                            _ => TextPredicate::LuaPattern {
                                source: 0,
                                compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                            },
                        };
                        let mut words = vec![word(SAVE_CURSOR, 0, 7), word(REPEAT_BEGIN, 0, 0)];
                        if !empty {
                            words.push(word(CHILD, 1, 0));
                            for index in 0..70 {
                                if index != 0 {
                                    words.push(word(NEXT, 1, 0));
                                }
                                words.extend([word(CAPTURE, 0, 0), word(CAPTURE, 0, 0)]);
                            }
                        }
                        words.extend([
                            word(ASSERT, 0, 0),
                            word(REPEAT_BUMP, 0, 0),
                            word(REPEAT_END, 0, 0),
                            word(RESTORE_CURSOR, 0, 7),
                            word(CAPTURE, 0, 1),
                            word(EMIT, 0, 2),
                        ]);
                        programs.push(
                            Program::new_with_capture_predicates(
                                words,
                                vec![
                                    pattern.into_bytes(),
                                    b"argument".to_vec(),
                                    b"root".to_vec(),
                                    b"match".to_vec(),
                                ],
                                vec![predicate],
                                vec![CapturePredicate::Text {
                                    capture: 0,
                                    predicate: 0,
                                    negated,
                                    any,
                                }],
                                vec![1, 2, 3],
                                vec![],
                            )
                            .unwrap(),
                        );
                    }
                }
            }
        }
        let files = [
            tape(&format!("{full}\n").repeat(70)),
            tape(&(format!("{full}\n").repeat(69) + "# b\n")),
        ];
        let anchors = files
            .iter()
            .map(|_| vec![vec![0]; programs.len()])
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        for value in 0..4 {
            assert_eq!(expected[0][value * 4].len(), 1);
            assert_eq!(expected[0][value * 4][0].captures.len(), 141);
            assert!(expected[1][value * 4].is_empty());
            assert_eq!(expected[1][value * 4 + 1].len(), 1);
            assert!(expected[1][value * 4 + 2].is_empty());
            assert_eq!(expected[1][value * 4 + 3].len(), 1);
        }
        assert!(
            expected
                .iter()
                .all(|file| file[16..].iter().all(Vec::is_empty))
        );
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_captured_membership_contains_and_ancestors_yield_with_exact_quantifiers() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CapturePredicate};
        let language = SupportLang::Python.get_ts_language();
        let good = format!("{}Z", "a".repeat(129));
        let strings = vec![
            b"item".to_vec(),
            b"no".to_vec(),
            good.as_bytes().to_vec(),
            b"Z".to_vec(),
            vec![],
            format!("{}Z", "a".repeat(65)).into_bytes(),
        ];
        let mut members = vec![1; 128];
        members.push(2);
        let mut programs = Vec::new();
        for empty in [false, true] {
            for contains in [false, true] {
                for any in [false, true] {
                    for negated in [false, true] {
                        let predicate = if contains {
                            CapturePredicate::Contains {
                                capture: 0,
                                strings: vec![5, 3, 4],
                                negated,
                                any,
                            }
                        } else {
                            CapturePredicate::AnyOf {
                                capture: 0,
                                strings: members.clone(),
                                negated,
                                any,
                            }
                        };
                        let mut words = if empty {
                            vec![]
                        } else {
                            vec![word(CAPTURE, 0, 0); 2]
                        };
                        words.extend([word(ASSERT, 0, 0), word(EMIT, 0, 0)]);
                        programs.push(
                            Program::new_with_capture_predicates(
                                words,
                                strings.clone(),
                                vec![],
                                vec![predicate],
                                vec![0],
                                vec![],
                            )
                            .unwrap(),
                        );
                    }
                }
            }
        }
        let module =
            Program::compile_tree_sitter_query_pattern("(module) @item", &language).unwrap();
        for values in [vec![2], members.clone()] {
            let mut query = module.clone();
            query.strings = strings.clone();
            programs.push(
                query
                    .with_capture_predicates(vec![CapturePredicate::AnyOf {
                        capture: 0,
                        strings: values,
                        negated: false,
                        any: false,
                    }])
                    .unwrap(),
            );
        }
        let identifier =
            Program::compile_tree_sitter_query_pattern("(identifier) @item", &language).unwrap();
        let mut kinds = vec![language.id_for_node_kind("comment", true); 128];
        kinds.push(language.id_for_node_kind("module", true));
        for ancestor in [false, true] {
            for negated in [false, true] {
                programs.push(
                    identifier
                        .clone()
                        .with_capture_predicates(vec![CapturePredicate::Relation {
                            capture: 0,
                            kinds: kinds.clone(),
                            ancestor,
                            negated,
                        }])
                        .unwrap(),
                );
            }
        }
        for negated in [false, true] {
            programs.push(
                Program::new_with_capture_predicates(
                    vec![word(ASSERT, 0, 0), word(EMIT, 0, 0)],
                    strings.clone(),
                    vec![],
                    vec![CapturePredicate::Relation {
                        capture: 0,
                        kinds: vec![],
                        ancestor: true,
                        negated,
                    }],
                    vec![0],
                    vec![],
                )
                .unwrap(),
            );
        }
        for contains in [false, true] {
            for negated in [false, true] {
                let predicate = if contains {
                    CapturePredicate::Contains {
                        capture: 0,
                        strings: vec![],
                        negated,
                        any: false,
                    }
                } else {
                    CapturePredicate::AnyOf {
                        capture: 0,
                        strings: vec![],
                        negated,
                        any: false,
                    }
                };
                programs.push(
                    Program::new_with_capture_predicates(
                        vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
                        strings.clone(),
                        vec![],
                        vec![predicate],
                        vec![0],
                        vec![],
                    )
                    .unwrap(),
                );
            }
        }
        let files = [
            tape(&good),
            tape(&format!("{}Y", "a".repeat(129))),
            tape(""),
            tape(&format!("value = {}x{}\n", "(".repeat(70), ")".repeat(70))),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        if program.tree_sitter_query {
                            (0..file.transducer_len() as u32).collect()
                        } else {
                            vec![0]
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), 1);
        assert_eq!(expected[0][4].len(), 1);
        assert!(expected[1][4].is_empty());
        assert_eq!(expected[1][6].len(), 1);
        assert_eq!(expected[1][7].len(), 1);
        assert_eq!(expected[0][12].len(), 1);
        assert!(expected[0][13].is_empty());
        assert_eq!(expected[3][20].len(), 2);
        assert!(expected[3][21].is_empty());
        assert_eq!(expected[0][22].len(), 1);
        assert!(expected[0][23].is_empty());
        assert!(expected[0][24].is_empty());
        assert_eq!(expected[0][25].len(), 1);
        assert!(expected[0][26].is_empty());
        assert!(expected[0][27].is_empty());
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_captured_kinds_and_lines_use_shared_bounded_argument_units() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CapturePredicate};
        let language = SupportLang::Python.get_ts_language();
        let module = language.id_for_node_kind("module", true);
        let identifier = language.id_for_node_kind("identifier", true);
        let mut kinds = vec![identifier; 128];
        kinds.push(module);
        let mut programs = Vec::new();
        for empty in [false, true] {
            for negated in [false, true] {
                let mut words = if empty {
                    vec![]
                } else {
                    vec![word(CAPTURE, 0, 0); 70]
                };
                words.extend([word(ASSERT, 0, 0), word(EMIT, 0, 0)]);
                programs.push(
                    Program::new_with_capture_predicates(
                        words,
                        vec![b"left".to_vec(), b"right".to_vec()],
                        vec![],
                        vec![CapturePredicate::Kinds {
                            capture: 0,
                            kinds: kinds.clone(),
                            negated,
                        }],
                        vec![0, 1],
                        vec![],
                    )
                    .unwrap(),
                );
            }
            for any in [false, true] {
                for negated in [false, true] {
                    let mut words = if empty {
                        vec![]
                    } else {
                        vec![word(CAPTURE, 0, 0); 70]
                    };
                    words.extend([word(ASSERT, 0, 0), word(EMIT, 0, 0)]);
                    programs.push(
                        Program::new_with_capture_predicates(
                            words,
                            vec![b"left".to_vec()],
                            vec![],
                            vec![CapturePredicate::Kind {
                                capture: 0,
                                kind: module,
                                negated,
                                any,
                            }],
                            vec![0],
                            vec![],
                        )
                        .unwrap(),
                    );
                }
            }
        }
        let base = Program :: compile_tree_sitter_query_pattern ("(assignment left: (identifier) @left right: [(identifier) @right (parenthesized_expression (identifier) @right)])" , & language) . unwrap () ;
        for negated in [false, true] {
            programs.push(
                base.clone()
                    .with_capture_predicates(vec![CapturePredicate::SameLine {
                        left: 0,
                        right: 1,
                        negated,
                    }])
                    .unwrap(),
            );
        }
        let files = [tape("x = x\nx = (\n x\n)\n")];
        let anchors = [programs
            .iter()
            .map(|program| {
                if program.tree_sitter_query {
                    (0..files[0].transducer_len() as u32).collect()
                } else {
                    vec![0]
                }
            })
            .collect::<Vec<_>>()];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), 1);
        assert_eq!(expected[0][0][0].captures.len(), 70);
        assert!(expected[0][1].is_empty());
        assert_eq!(expected[0][6].len(), 1);
        assert_eq!(expected[0][7].len(), 1);
        assert_eq!(expected[0][12].len(), 1);
        assert_eq!(expected[0][13].len(), 1);
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_captured_equality_uses_bounded_shared_units_for_inline_and_late_predicates() {
        let language = SupportLang::Python.get_ts_language();
        let comment = language.id_for_node_kind("comment", true);
        let mut programs = Vec::new();
        for empty in [false, true] {
            for any in [false, true] {
                for negated in [false, true] {
                    let mut words = Vec::new();
                    if !empty {
                        words.push(word(crate::code_rule_dsl::transducer::packed::CHILD, 1, 0));
                        for index in 0..140 {
                            if index != 0 {
                                words.push(word(crate::code_rule_dsl::transducer::packed::NEXT, 1, 0));
                            }
                            words.push(word(crate::code_rule_dsl::transducer::packed::KIND, 0, comment));
                            words.push(word(CAPTURE, 0, index % 2));
                            words.push(word(CAPTURE, 0, index % 2));
                        }
                    }
                    words.extend([
                        word(crate::code_rule_dsl::transducer::packed::ASSERT, 0, 0),
                        word(EMIT, 0, 2),
                    ]);
                    programs.push(
                        Program::new_with_capture_predicates(
                            words,
                            vec![b"left".to_vec(), b"right".to_vec(), b"match".to_vec()],
                            vec![],
                            vec![crate::code_rule_dsl::transducer::packed::CapturePredicate::Equal {
                                left: 0,
                                right: 1,
                                negated,
                                any,
                            }],
                            vec![0, 1, 2],
                            vec![],
                        )
                        .unwrap(),
                    );
                }
            }
        }
        for predicate in ["eq?", "not-eq?", "any-eq?", "any-not-eq?"] {
            programs . push (Program :: compile_tree_sitter_query_pattern (& format ! ("((assignment left: (identifier) @left right: (identifier) @right) (#{predicate} @left @right))") , & language) . unwrap ()) ;
        }
        let mut unequal = programs[0].clone();
        unequal
            .words
            .insert(unequal.words.len() - 2, word(CAPTURE, 0, 0));
        programs.push(unequal);
        for (right, negated) in [(0, false), (2, true)] {
            programs.push(
                programs[0]
                    .clone()
                    .with_capture_predicates(vec![
                        crate::code_rule_dsl::transducer::packed::CapturePredicate::Equal {
                            left: 0,
                            right,
                            negated,
                            any: false,
                        },
                    ])
                    .unwrap(),
            );
        }
        let long = "a".repeat(129);
        let files = [
            tape(&(format!("#{long}\n").repeat(140) + &format!("{long} = {long}\n"))),
            tape(
                &(format!("#{long}\n").repeat(139)
                    + &format!("#{}b\n{long} = other\n", "a".repeat(128))),
            ),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        if program.tree_sitter_query {
                            (0..file.transducer_len() as u32).collect()
                        } else {
                            vec![0]
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            expected
                .iter()
                .map(|file| file.iter().map(Vec::len).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [
                vec![1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 1, 0],
                vec![0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0],
            ],
            "complete equality truth, including empty groups and unequal cardinalities"
        );
        assert_eq!(expected[0][0][0].captures.len(), 280);
        for (index, pair) in expected[0][0][0].captures.chunks_exact(2).enumerate() {
            assert_eq!(pair[0], pair[1], "duplicate capture {index}");
            assert_eq!(pair[0].label, (index % 2) as u16);
            assert_eq!(pair[0].span, index * 131..index * 131 + 130);
        }
        let native =
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap();
        assert_eq!(native, expected);
        let scope = metal_backend::launch::LaunchScope::enter();
        let started = std::time::Instant::now();
        let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
        let elapsed = started.elapsed().as_secs_f64();
        let trace = scope.snapshot();
        eprintln!(
            "LONG_CAPTURE_ATTEMPT {}",
            serde_json::json!({
                "attempts":trace.len(),"encoded":trace.iter().filter(|r|r.encoded).count(),
                "completed":trace.iter().filter(|r|r.completed).count(),"seconds":elapsed,
            })
        );
        assert_eq!(actual, Ok(expected));
        assert!(trace.len() <= 600);
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        eprintln!(
            "LONG_CAPTURE_EVIDENCE {}",
            serde_json::json!({
                "programs":programs.len(),"files":files.len(),"launches":trace.len(),"seconds":elapsed,
                "peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),
            })
        );
    }
    #[test]
    fn inputs_scalar_lua_attempts_bound_scans_and_defer_without_semantic_errors() {
        let cases = [
            ("^Z", "a".repeat(1025), false),
            ("^a", "a".repeat(1025), false),
            ("^a$", "a".repeat(1025), false),
            ("^a?", "a".repeat(1025), false),
            ("^a-", "a".repeat(1025), false),
            ("^a*", "a".repeat(1025), false),
            ("^.*$", "a\né".repeat(257), false),
            ("^.+$", "a".repeat(1025), false),
            ("^.+$", String::new(), false),
            ("^a+$", "a".repeat(1025), true),
            ("Z", format!("{}Z", "a".repeat(129)), true),
            ("^%b()$", format!("({})", "a".repeat(129)), true),
            ("^%b()$", "(a)".to_owned(), false),
            ("^a+b$", "aaab".to_owned(), false),
            ("^a+b$", "aaac".to_owned(), false),
            ("^a-b$", "aaaab".to_owned(), false),
            ("^(ab)%1$", "abab".to_owned(), false),
            ("^.-$", "aaaa".to_owned(), false),
            ("^.-$", "a".repeat(129), true),
            ("^(a+)%1$", "a".repeat(130), true),
            (
                "^(abcdefghijklmnop)%1%1%1$",
                "abcdefghijklmnop".repeat(4),
                true,
            ),
        ];
        let programs = cases
            .iter()
            .map(|(pattern, _, _)| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.as_bytes().to_vec(), b"match".to_vec()],
                    vec![TextPredicate::LuaPattern {
                        source: 0,
                        compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = cases
            .iter()
            .map(|(_, source, _)| tape(source))
            .collect::<Vec<_>>();
        let anchors = (0..cases.len())
            .map(|file| {
                (0..cases.len())
                    .map(|program| if file == program { vec![0] } else { vec![] })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_linear_lua_blocks_preserve_search_cursors_and_exact_cpu_work_limits() {
        let mut cases = vec![
            ("[^\n ]%)$".to_owned(), format!("{})", "a".repeat(63))),
            ("[^\n ]%)$".to_owned(), format!("{})", "a".repeat(65))),
            ("[^\n ]%)$".to_owned(), "a".repeat(129)),
            ("ab$".to_owned(), format!("{}b", "a".repeat(129))),
            ("^ab".to_owned(), format!("ab{}", "a".repeat(129))),
            ("^ab$".to_owned(), format!("ab{}", "a".repeat(129))),
            (String::new(), "a".repeat(65)),
            ("$".to_owned(), "a".repeat(129)),
            ("^$".to_owned(), "a".repeat(65)),
            (
                format!("{}Z", "a".repeat(31)),
                format!("{}Z", "a".repeat(129)),
            ),
            ("[^ ]%)$".to_owned(), format!("{}é)", "a".repeat(64))),
        ];
        cases.extend((61..=65).map(|length| ("Z".to_owned(), format!("{}Z", "a".repeat(length)))));
        for (pattern, source) in cases {
            let programs = [Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                vec![TextPredicate::LuaPattern {
                    source: 0,
                    compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                }],
                vec![1],
                vec![],
            )
            .unwrap()];
            let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Lua(lua) =
                &programs[0].value_programs[0]
            else {
                unreachable!()
            };
            let (_, work) = lua.test_work_count(source.as_bytes());
            assert!(
                work > 0,
                "the retained cost oracle must account for this input"
            );
            let files = [tape(&source)];
            let anchors = [vec![vec![0]]];
            for limit in [work.saturating_sub(1), work, work + 1]
                .into_iter()
                .filter(|limit| *limit != 0)
            {
                assert_value_budget(&programs[0], &source, limit, false);
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_runtime_deferred_root_predicates_use_exact_counts_at_page_boundaries() {
        let language = SupportLang::Python.get_ts_language();
        let programs = [
            r#"((identifier) @name (#match? @name "^a+$"))"#,
            r#"((identifier) @name (#lua-match? @name "^a+$"))"#,
            r#"((identifier) @name (#not-match? @name "^a+$"))"#,
        ]
        .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap());
        let source = (0..12)
            .map(|i| format!("{}\n", "a".repeat(65 + i * 7)))
            .collect::<String>()
            + "short\n";
        let files = [tape(&source)];
        let anchors = [vec![
            (0..files[0].transducer_len() as u32)
                .collect::<Vec<_>>();
            programs.len()
        ]];
        {
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_small_automata_defer_long_inputs_into_shared_value_frontiers() {
        let language = SupportLang::Python.get_ts_language();
        let mut programs = [
            r#"((identifier) @name (#match? @name "^a+$"))"#,
            r#"((identifier) @name (#not-match? @name "^a+$"))"#,
            r#"((identifier) @name (#lua-match? @name "^a+$"))"#,
            r#"((assignment right: (identifier) @name) (#match? @name "^a+$"))"#,
        ]
        .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
        .to_vec();
        for lua in [false, true] {
            let pattern = "^a+$";
            programs.push(
                Program::new(
                    vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.as_bytes().to_vec(), b"name".to_vec()],
                    vec![if lua {
                        TextPredicate::LuaPattern {
                            source: 0,
                            compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                        }
                    } else {
                        TextPredicate::Regex {
                            source: 0,
                            compiled: regex::Regex::new(pattern).unwrap(),
                        }
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap(),
            );
        }
        let files = [
            tape(&"a".repeat(65)),
            tape(&format!("# shifted\nvalue = {}b\n", "a".repeat(129))),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_shared_root_predicates_intern_values_and_need_no_frontier_records() {
        let language = SupportLang::Python.get_ts_language();
        let queries = [
            r#"((identifier) @name (#match? @name "^a"))"#,
            r#"((identifier) @name (#not-match? @name "^a"))"#,
            r#"((identifier) @name (#lua-match? @name "^b%w*$"))"#,
            r#"((assignment left: (identifier) @name) (#match? @name "^a"))"#,
            r#"((assignment right: (identifier) @name) (#not-match? @name "^a"))"#,
            r#"((assignment left: (identifier) @name) (#eq? @name "alpha"))"#,
            r#"((assignment left: (identifier) @name) (#any-of? @name "alpha" "again"))"#,
        ];
        let programs = queries
            .iter()
            .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        let files = [
            tape("alpha = beta\nother = alpha\n"),
            tape("beta = again\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_small_register_nfa_work_and_budget_boundaries_match_cpu_order() {
        let patterns = [
            "(?:a?)*z",
            "a|ab",
            "^a+$",
            r"^_*[A-Z][A-Z\d_]*$",
            "^[A-Z][A-Z_]*$",
            "",
            r"\bzz\b",
            "(?:ab|bc|cd|de|ef|fg|gh|hi|ij|jk|kl|lm|mn|no|op)",
        ];
        let source = "aaab zzab BinaryIO __name__ _A T2 IO";
        let programs = patterns
            .iter()
            .map(|pattern| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.as_bytes().to_vec(), b"match".to_vec()],
                    vec![TextPredicate::Regex {
                        source: 0,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = [tape(source)];
        let anchors = [programs.iter().map(|_| vec![0]).collect::<Vec<_>>()];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_small_nfa_blocks_preserve_queues_and_work_at_byte_boundaries() {
        for pattern in [
            "^a+$".to_owned(),
            format!("^{}Z$", "a*".repeat(20)),
            "^[aé]+$".to_owned(),
        ] {
            let programs = [Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                vec![TextPredicate::Regex {
                    source: 0,
                    compiled: regex::Regex::new(&pattern).unwrap(),
                }],
                vec![1],
                vec![],
            )
            .unwrap()];
            let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Regex(nfa) =
                &programs[0].value_programs[0]
            else {
                unreachable!()
            };
            for source in [
                "a".repeat(63),
                "a".repeat(64),
                "a".repeat(65),
                format!("{}Z", "a".repeat(129)),
                format!("{}é", "a".repeat(63)),
            ] {
                let (_, work) = nfa.test_work_count(source.as_bytes());
                let files = [tape(&source)];
                let anchors = [vec![vec![0]]];
                for limit in [work - 1, work, work + 1] {
                    assert_value_budget(&programs[0], &source, limit, false);
                    assert_fixture(&programs, &files, &anchors);
                }
            }
        }
    }
    #[test]
    fn inputs_wide_nfa_splits_preserve_branch_order_and_exact_work_limits() {
        let pattern = format!(
            "^(?:{})$",
            (1..=130)
                .map(|n| format!("a{{{n}}}b"))
                .collect::<Vec<_>>()
                .join("|")
        );
        let programs = [Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
            vec![TextPredicate::Regex {
                source: 0,
                compiled: regex::Regex::new(&pattern).unwrap(),
            }],
            vec![1],
            vec![],
        )
        .unwrap()];
        let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Regex(nfa) =
            &programs[0].value_programs[0]
        else {
            unreachable!()
        };
        assert ! (nfa . states . iter () . any (| state | matches ! (state , crate::code_rule_dsl::transducer::packed::value :: NfaState :: Split (next) if next . len () == 130)));
        for source in [
            "ab".to_owned(),
            format!("{}b", "a".repeat(130)),
            "x".to_owned(),
        ] {
            let files = [tape(&source)];
            let anchors = [vec![vec![0]]];
            for ordered in [false, true] {
                let (_, work) = if ordered {
                    let (matched, work) = nfa.test_find_work_count(source.as_bytes());
                    (matched.is_some(), work)
                } else {
                    let (matched, work) = nfa.test_work_count(source.as_bytes());
                    (matched.unwrap(), work)
                };
                for limit in [work - 1, work, work + 1] {
                    assert_value_budget(&programs[0], &source, limit, ordered);
                    assert_fixture(&programs, &files, &anchors);
                }
            }
        }
    }
    #[test]
    fn inputs_compact_stage_nfa_work_counts_match_the_cpu_oracle_at_budget_boundaries() {
        let prefix = "p".repeat(80);
        let source = format!("zzz{prefix}aaab");
        for suffix in ["a*", "a*?", "(?:a?)*", "(?:a|aa)*Z", "(a|aa)", "(aa|a)"] {
            let pattern = format!("{prefix}{suffix}");
            let programs = [Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                vec![TextPredicate::Regex {
                    source: 0,
                    compiled: regex::Regex::new(&pattern).unwrap(),
                }],
                vec![1],
                vec![],
            )
            .unwrap()];
            let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Regex(nfa) =
                &programs[0].value_programs[0]
            else {
                unreachable!()
            };
            let (result, work) = nfa.test_work_count(source.as_bytes());
            assert!(result.is_ok());
            let files = [tape(&source)];
            let anchors = [vec![vec![0]]];
            for limit in [work - 1, work, work + 1] {
                assert_value_budget(&programs[0], &source, limit, false);
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_compact_stage_ordered_nfa_work_matches_cpu_at_budget_boundaries() {
        let prefix = "p".repeat(80);
        let source = format!("zzz{prefix}aaab{prefix}aaa");
        for suffix in ["a*", "a*?", "(a|aa)", "(aa|a)", "(a|aa)*", "(?:a?)*", "z+"] {
            let pattern = format!("{prefix}{suffix}");
            let regex = regex::Regex::new(&pattern).unwrap();
            let expected = regex
                .find(&source)
                .map(|found| [found.start() as u32, found.end() as u32]);
            let programs = [Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                vec![TextPredicate::Regex {
                    source: 0,
                    compiled: regex,
                }],
                vec![1],
                vec![],
            )
            .unwrap()];
            let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Regex(nfa) =
                &programs[0].value_programs[0]
            else {
                unreachable!()
            };
            let (actual, work) = nfa.test_find_work_count(source.as_bytes());
            assert_eq!(actual, expected, "ordered CPU oracle: {pattern}");
            let files = [tape(&source)];
            let anchors = [vec![vec![0]]];
            for limit in [work - 1, work, work + 1] {
                assert_value_budget(&programs[0], &source, limit, true);
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_compact_stage_predicate_history_recycles_past_input_positions() {
        let source = "a".repeat(300);
        let pattern = "^a{300}$";
        let programs = [Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
            vec![TextPredicate::Regex {
                source: 0,
                compiled: regex::Regex::new(pattern).unwrap(),
            }],
            vec![1],
            vec![],
        )
        .unwrap()];
        let files = [tape(&source)];
        let anchors = [vec![vec![0]]];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), 1);
        let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Regex(nfa) =
            &programs[0].value_programs[0]
        else {
            unreachable!()
        };
        let (result, work) = nfa.test_work_count(source.as_bytes());
        assert_eq!(result, Ok(true));
        assert_value_budget(&programs[0], &source, work, false);
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_capturing_sibling_closures_preserve_maximal_prefixes() {
        let patterns = [
            "(module ((comment) @item)+)",
            "(module ((comment) @item)+ (comment) @last)",
            "(module ((comment) @item)*)",
            "(module ((comment) @one @two)+)",
            "(module [(comment) @item (expression_statement)]+)",
            "(module [(comment) @item (expression_statement) @value]+)",
            "(module [(comment) @item (expression_statement) @value (function_definition) @function]+)",
            "(module [(comment) @left (comment) @right]+)",
        ];
        let programs = patterns
            .iter()
            .map(|query| {
                Program::compile_tree_sitter_query_pattern(
                    query,
                    &SupportLang::Python.get_ts_language(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        for count in 0..=8 {
            let source = (0..count).map(|i| format!("# {i}\n")).collect::<String>();
            let files = [tape(&source), tape("# one\nvalue\n# two\nother\n# three\n")];
            let anchors = files
                .iter()
                .map(|file| {
                    programs
                        .iter()
                        .map(|_| (0..file.transducer_len() as u32).collect())
                        .collect()
                })
                .collect::<Vec<Vec<Vec<u32>>>>();
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_sibling_closure_chunks_preserve_captures_and_order_across_unmatched_gaps() {
        let programs = [
            "(module [(comment) @item (expression_statement) @value]+)",
            "(module [(comment) @item (expression_statement)]+)",
        ]
        .map(|query| {
            Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap()
        });
        for count in [63, 64, 65, 129] {
            let files = [
                tape(&format!(
                    "# first\n{}# last\nvalue\n",
                    "pass\n".repeat(count)
                )),
                tape(&"# item\nvalue\n".repeat(count / 2 + 1)),
            ];
            let anchors = files
                .iter()
                .map(|_| vec![vec![0]; programs.len()])
                .collect::<Vec<_>>();
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_compact_stage_query_large_values_follow_maximal_capture_selection() {
        let pattern = "x".repeat(80);
        let queries = [
            format!("((comment) @text (#match? @text \"{pattern}\"))"),
            format!("((comment) @text (#not-match? @text \"{pattern}\"))"),
            format!("((comment) @text (#lua-match? @text \"{pattern}\"))"),
            format!("((module (comment)+ @text) (#any-match? @text \"{pattern}\"))"),
            format!("((module (comment)+ @text) (#not-match? @text \"{pattern}\"))"),
            format!("((comment) @text (#match? @text \"{pattern}\") (#eq? @text \"# {pattern}\"))"),
        ];
        let programs = queries
            .iter()
            .map(|query| {
                Program::compile_tree_sitter_query_pattern(
                    query,
                    &SupportLang::Python.get_ts_language(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = [
            tape(&format!("# {pattern}\n# short\n# {pattern}\n")),
            tape("# short\n# short\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_large_capture_values_preserve_quantifiers_and_negation() {
        let pattern = "x".repeat(80);
        for lua in [false, true] {
            let programs = [false, true]
                .into_iter()
                .flat_map(|negated| [false, true].into_iter().map(move |any| (negated, any)))
                .map(|(negated, any)| {
                    let predicate = if lua {
                        TextPredicate::LuaPattern {
                            source: 0,
                            compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                        }
                    } else {
                        TextPredicate::Regex {
                            source: 0,
                            compiled: regex::Regex::new(&pattern).unwrap(),
                        }
                    };
                    Program::new_with_capture_predicates(
                        vec![
                            word(CHILD, 1, 0),
                            word(CAPTURE, 0, 0),
                            word(NEXT, 1, 0),
                            word(CAPTURE, 0, 0),
                            word(ASSERT, 0, 0),
                            word(EMIT, 0, 0),
                        ],
                        vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                        vec![predicate],
                        vec![CapturePredicate::Text {
                            capture: 0,
                            predicate: 0,
                            negated,
                            any,
                        }],
                        vec![1],
                        vec![],
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>();
            let files = [
                tape(&format!("# {pattern}\n# short\n# {pattern}\n")),
                tape(&format!("# {pattern}\n# {pattern}\n")),
                tape("# short\n# short\n"),
            ];
            let anchors = files
                .iter()
                .map(|_| programs.iter().map(|_| vec![0]).collect())
                .collect::<Vec<Vec<Vec<u32>>>>();
            let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
            assert_eq!(
                expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
                [0, 2, 0, 2]
            );
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_compact_stage_lua_work_counts_match_the_cpu_oracle_at_budget_boundaries() {
        let prefix = "x".repeat(40);
        for (text, suffix) in [
            ("aaab", "a*b"),
            ("aaab", "a-b"),
            ("aaab", "a+b"),
            ("b", "a?b"),
            ("aaab", "a*a*Z"),
            ("(a(b)c)", "%b()"),
            ("(abc", "%b()"),
            ("nana", "(%a+)%1"),
            ("abcdeabcde", "(%a)(%a)(%a)(%a)(%a)%1%2%3%4%5"),
        ] {
            let source = format!("zz{prefix}{text}");
            let pattern = format!("{prefix}{suffix}");
            let programs = [Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                vec![TextPredicate::LuaPattern {
                    source: 0,
                    compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                }],
                vec![1],
                vec![],
            )
            .unwrap()];
            let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Lua(lua) =
                &programs[0].value_programs[0]
            else {
                unreachable!()
            };
            let (result, work) = lua.test_work_count(source.as_bytes());
            assert!(result.is_ok());
            let files = [tape(&source)];
            let anchors = [vec![vec![0]]];
            for limit in [work - 1, work, work + 1] {
                assert_value_budget(&programs[0], &source, limit, false);
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_compact_stage_large_lua_uses_shared_capture_and_pending_frontiers() {
        let prefix = "x".repeat(40);
        let cases = [
            ("abc123", "%a+%d+"),
            ("(a(b)c)", "%b()"),
            ("nana", "(%a+)%1"),
            (" name", " %f[%a]name"),
            ("name", "()name"),
            ("aaab", "a.-b"),
            ("b", "a?b"),
            ("aaab", "a*b"),
            ("aaab", "a+b"),
            ("abcdeabcde", "(%a)(%a)(%a)(%a)(%a)%1%2%3%4%5"),
        ];
        let programs = cases
            .iter()
            .map(|(_, suffix)| {
                let pattern = format!("{prefix}{suffix}");
                Program::new(
                    vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                    vec![TextPredicate::LuaPattern {
                        source: 0,
                        compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = cases
            .iter()
            .map(|(suffix, _)| tape(&format!("# {prefix}{suffix}\n")))
            .chain([tape("# no match\n")])
            .collect::<Vec<_>>();
        let anchors = files
            .iter()
            .enumerate()
            .map(|(file, _)| {
                programs
                    .iter()
                    .enumerate()
                    .map(|(program, _)| {
                        if file == program || file == cases.len() {
                            vec![0]
                        } else {
                            vec![]
                        }
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        for (index, file) in expected.iter().enumerate().take(cases.len()) {
            assert!(!file[index].is_empty(), "case {index}");
        }
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_large_regex_uses_shared_nfa_frontiers() {
        let long = "a".repeat(80);
        let patterns = [
            format!("^{long}$"),
            format!("({long}|{}b)", "a".repeat(79)),
            format!("({long})+"),
            format!(r"\b{long}\b"),
        ];
        let programs = patterns
            .iter()
            .map(|pattern| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.as_bytes().to_vec(), b"text".to_vec()],
                    vec![TextPredicate::Regex {
                        source: 0,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = [
            tape(&format!("{long}\n")),
            tape(&format!("{}b\n", "a".repeat(79))),
            tape("short\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_cursor_cache_splits_large_layouts_and_keeps_sparse_slot_ids() {
        let mut words = vec![word(CHILD, 1, 0)];
        for slot in 0..64 {
            words.extend([word(SAVE_CURSOR, 0, slot), word(NEXT, 1, 0)]);
        }
        for slot in (0..64).rev() {
            words.extend([word(RESTORE_CURSOR, 0, slot), word(CAPTURE, 0, 0)]);
        }
        words.push(word(EMIT, 0, 0));
        let programs =
            [Program::new(words, vec![b"comment".to_vec()], vec![], vec![0], vec![]).unwrap()];
        let source = (0..70)
            .map(|index| format!("# {index}\n"))
            .collect::<String>();
        let files = [tape(&source)];
        let anchors = [vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_wide_cursor_layouts_use_bounded_persistent_updates_with_cpu_parity() {
        for count in [65u16, 129] {
            let mut words = vec![word(CHILD, 1, 0)];
            for slot in 0..count {
                words.extend([word(SAVE_CURSOR, 0, slot), word(NEXT, 1, 0)]);
            }
            for slot in (0..count).rev() {
                words.extend([word(RESTORE_CURSOR, 0, slot), word(CAPTURE, 0, 0)]);
            }
            words.push(word(EMIT, 0, 0));
            let programs =
                [
                    Program::new(words, vec![b"comment".to_vec()], vec![], vec![0], vec![])
                        .unwrap(),
                ];
            let source = (0..u32::from(count) + 2)
                .map(|index| format!("# {index}\n"))
                .collect::<String>();
            let files = [tape(&source)];
            let anchors = [vec![vec![0]]];
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_compact_stage_generated_cursor_registers_preserve_all_sixteen_slots() {
        let mut words = vec![word(CHILD, 1, 0)];
        for slot in 0..16 {
            words.push(word(SAVE_CURSOR, 0, slot));
            words.push(word(NEXT, 1, 0));
        }
        for slot in (0..16).rev() {
            words.push(word(RESTORE_CURSOR, 0, slot));
            words.push(word(CAPTURE, 0, 0));
        }
        words.push(word(EMIT, 0, 0));
        let programs =
            [Program::new(words, vec![b"comment".to_vec()], vec![], vec![0], vec![]).unwrap()];
        let source = (0..20)
            .map(|index| format!("# {index}\n"))
            .collect::<String>();
        let files = [tape(&source)];
        let anchors = vec![vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_repeated_strip_shares_one_work_budget() {
        let prefix = "p".repeat(80);
        let pattern = format!("{prefix}a*");
        let source = format!("{prefix}aaa").repeat(3);
        let nfa = crate::code_rule_dsl::transducer::packed::value::ByteNfa::compile(&pattern).unwrap();
        let mut search = 0;
        let mut work = 0;
        let mut finds = 0;
        loop {
            let (found, cost) = nfa.test_find_work_count(&source.as_bytes()[search..]);
            work += cost;
            finds += 1;
            let Some([_, end]) = found else { break };
            assert!(end > 0);
            search += end as usize;
        }
        assert_eq!(finds, 4);
        let programs = [Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_directives(vec![Directive::Strip {
            capture: 0,
            source: 1,
            compiled: regex::Regex::new(&pattern).unwrap(),
        }])
        .unwrap()];
        let files = [tape(&source)];
        let anchors = [vec![vec![0]]];
        for limit in [work - 1, work, work + 1] {
            assert_strip_budget(&pattern, &source, limit);
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_compact_stage_large_repeated_strip_matches_cpu_across_typed_pages() {
        let base = Program::compile_tree_sitter_query_pattern(
            "(comment) @text",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let mut programs = Vec::new();
        for (first, second) in [
            ("(?:a|aa|z{80})", "n"),
            ("a*", ""),
            ("a+?", "é"),
            (r"\b", "A"),
        ] {
            for downcase_first in [false, true] {
                let mut program = base.clone();
                let mut directives = Vec::new();
                if downcase_first {
                    directives.push(Directive::Downcase { capture: 0 });
                }
                for pattern in [first, second] {
                    let source = program.strings.len() as u16;
                    program.strings.push(pattern.as_bytes().to_vec());
                    directives.push(Directive::Strip {
                        capture: 0,
                        source,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    });
                }
                if !downcase_first {
                    directives.push(Directive::Downcase { capture: 0 });
                }
                programs.push(program.with_directives(directives).unwrap());
            }
        }
        programs.push(
            Program::new(
                vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"text".to_vec(), b"a".to_vec(), b"n".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![
                Directive::Strip {
                    capture: 0,
                    source: 1,
                    compiled: regex::Regex::new("a").unwrap(),
                },
                Directive::Strip {
                    capture: 0,
                    source: 2,
                    compiled: regex::Regex::new("n").unwrap(),
                },
            ])
            .unwrap(),
        );
        let files = [
            tape("# AAA banana aaab Éé\n# second banana\n"),
            tape("# aaaa banana\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_ordered_strip_preserves_priority_empty_matches_and_transform_order() {
        let base = Program::compile_tree_sitter_query_pattern(
            "(comment) @text",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let files = [tape("# banana aaab Éé\n"), tape("# AAA banana\n")];
        for pattern in ["n", "a|aa", "aa|a", "a*", "a+?", "", r"\b", "é", r"^#\s*"] {
            for downcase_first in [false, true] {
                let mut program = base.clone();
                let source = program.strings.len() as u16;
                program.strings.push(pattern.as_bytes().to_vec());
                let strip = Directive::Strip {
                    capture: 0,
                    source,
                    compiled: regex::Regex::new(pattern).unwrap(),
                };
                let lower = Directive::Downcase { capture: 0 };
                let directives = if downcase_first {
                    vec![lower, strip]
                } else {
                    vec![strip, lower]
                };
                let programs = [program.with_directives(directives).unwrap()];
                let anchors = files
                    .iter()
                    .map(|file| vec![(0..file.transducer_len() as u32).collect()])
                    .collect::<Vec<Vec<Vec<u32>>>>();
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_compact_stage_adjacent_selection_preserves_directive_order() {
        let program = Program::new(
            vec![
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 1, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 1, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![
                b"comment".to_vec(),
                b"target".to_vec(),
                b"before".to_vec(),
                b"value".to_vec(),
                b"after".to_vec(),
            ],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_directives(vec![
            Directive::SetCapture {
                capture: 0,
                key: 2,
                value: Some(3),
            },
            Directive::SelectAdjacent {
                capture: 0,
                target: 1,
            },
            Directive::Downcase { capture: 0 },
            Directive::SelectAdjacent {
                capture: 0,
                target: 1,
            },
            Directive::SetCapture {
                capture: 0,
                key: 4,
                value: Some(3),
            },
        ])
        .unwrap();
        let programs = [program];
        let files = [
            tape("# FAR\n\n# CLOSE\nvalue\n"),
            tape("# FIRST\n# SECOND\nvalue\n"),
        ];
        let anchors = vec![vec![vec![0]], vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
        let compiled_strip = regex::Regex::new("# ").unwrap();
        for position in [0, 2, 5] {
            let mut program = programs[0].clone();
            let source = program.strings.len() as u16;
            program.strings.push(b"# ".to_vec());
            let mut directives = program.directives.clone();
            directives.insert(
                position,
                Directive::Strip {
                    capture: 0,
                    source,
                    compiled: compiled_strip.clone(),
                },
            );
            assert_fixture(
                &[program.with_directives(directives).unwrap()],
                &files,
                &anchors,
            );
        }
    }
    #[test]
    fn inputs_compact_stage_transformed_bytes_and_mime_properties_cross_pages() {
        let program = Program::compile_tree_sitter_query_pattern(
            "(identifier) @text",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let programs = [
            program
                .clone()
                .with_directives(vec![
                    Directive::Downcase { capture: 0 },
                    Directive::SetLanguageFromMimeType { capture: 0 },
                ])
                .unwrap(),
            program
                .with_directives(vec![
                    Directive::Downcase { capture: 0 },
                    Directive::Offset {
                        capture: 0,
                        offsets: [0, 1, 0, -1],
                    },
                    Directive::SetLanguageFromMimeType { capture: 0 },
                ])
                .unwrap(),
        ];
        let files = [tape("MODULE\n"), tape("module\n"), tape("importmap\n")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_typed_publication_preserves_raw_dropped_capture_properties_and_byte_order() {
        let program = Program::new(
            vec![
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 1, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 1, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![
                b"comment".to_vec(),
                b"target".to_vec(),
                b"before".to_vec(),
                b"value".to_vec(),
                b"after".to_vec(),
                b"# ".to_vec(),
                b"match".to_vec(),
                b"targetProp".to_vec(),
            ],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_directives(vec![
            Directive::Set {
                key: 6,
                value: Some(3),
            },
            Directive::SetCapture {
                capture: 0,
                key: 2,
                value: Some(3),
            },
            Directive::Strip {
                capture: 0,
                source: 5,
                compiled: regex::Regex::new("# ").unwrap(),
            },
            Directive::SelectAdjacent {
                capture: 0,
                target: 1,
            },
            Directive::Downcase { capture: 0 },
            Directive::SetCapture {
                capture: 0,
                key: 4,
                value: Some(3),
            },
            Directive::SetCapture {
                capture: 1,
                key: 7,
                value: None,
            },
            Directive::Downcase { capture: 1 },
            Directive::SetLanguageFromMimeType { capture: 1 },
            Directive::Set {
                key: 2,
                value: None,
            },
        ])
        .unwrap();
        let programs = [program];
        let files = [tape("# FAR\n\n# CLOSE\nvalue\n")];
        let anchors = vec![vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_prepared_mime_prefixes_resume_over_long_source_and_keep_directive_order() {
        let base = Program::compile_tree_sitter_query_pattern(
            "(comment) @text",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let programs = [base
            .with_directives(vec![
                Directive::Offset {
                    capture: 0,
                    offsets: [0, 2, 0, 0],
                },
                Directive::Downcase { capture: 0 },
                Directive::SetLanguageFromMimeType { capture: 0 },
                Directive::Offset {
                    capture: 0,
                    offsets: [0, 0, 0, -1],
                },
                Directive::SetLanguageFromMimeType { capture: 0 },
            ])
            .unwrap()];
        let long = format!("# application/{}/PYTHON\n", "A".repeat(130));
        let files = [
            tape("# importmap\n"),
            tape("# module\n"),
            tape("# application/ecmascript\n"),
            tape("# text/ecmascript\n"),
            tape(&long),
        ];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect()])
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_direct_constant_properties_use_scanned_pages_without_candidate_storage() {
        let language = SupportLang::Python.get_ts_language();
        let programs = [
            Program::compile_tree_sitter_query_pattern(
                "((identifier) @name (#set! role \"name\") (#set! category \"symbol\"))",
                &language,
            )
            .unwrap(),
            Program::compile_tree_sitter_query_pattern(
                "((call function: (identifier) @name) (#set! role \"callee\"))",
                &language,
            )
            .unwrap(),
        ];
        let files = [tape("first(one)\nsecond(two)\n")];
        let anchors = vec![
            programs
                .iter()
                .map(|_| (0..files[0].transducer_len() as u32).collect())
                .collect(),
        ];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_output_properties_and_offsets_cross_pages() {
        let program = Program :: compile_tree_sitter_query_pattern ("((identifier) @inner @outer (#offset! @inner 0 1 0 -1) (#set! role \"trimmed\") (#set! @outer kind \"raw\"))" , & SupportLang :: Python . get_ts_language () ,) . unwrap () ;
        let programs = [program];
        let files = [tape("alpha = bravo\ncharlie = delta\n")];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_optional_nested_branches_and_quantified_strings_preserve_saved_context() {
        let queries = [
            "(if_statement alternative: (_ (_) @conditional.inner)?) @conditional.outer",
            "(call function: (attribute object: (identifier) @_re) arguments: (argument_list (string (string_start) [(string_content) @injection.content (interpolation)]+ (string_end))) (#eq? @_re \"re\") (#set! injection.language \"regex\"))",
            "(call function: (attribute object: (identifier) @_re) arguments: (argument_list (concatenated_string (string (string_start) [(string_content) @injection.content (interpolation)]+ (string_end)))) (#eq? @_re \"re\") (#set! injection.language \"regex\"))",
        ];
        let mut programs = queries
            .iter()
            .map(|query| {
                Program::compile_tree_sitter_query_pattern(
                    query,
                    &SupportLang::Python.get_ts_language(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        if let Ok(root) = std::env::var("TEMPO_CONTEXT_REPRO_CATALOG") {
            programs.clear();
            let mut paths = std::fs::read_dir(root)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "scm"))
                .collect::<Vec<_>>();
            paths.sort();
            for path in paths {
                for query in
                    Program::tree_sitter_top_level_patterns(&std::fs::read_to_string(path).unwrap())
                        .unwrap()
                {
                    programs.push(
                        Program::compile_tree_sitter_query_pattern(
                            &query,
                            &SupportLang::Python.get_ts_language(),
                        )
                        .unwrap(),
                    );
                }
            }
            assert_eq!(programs.len(), 335);
        }
        let mut files = vec![
            tape(
                "if one:\n    pass\nelif two:\n    pass\nelif three:\n    pass\nelse:\n    first()\n    second()\n",
            ),
            tape("re.compile(f'a{b}c' 'd')\nre.match('a' f'{b}c', text)\n"),
        ];
        if let Ok(root) = std::env::var("TEMPO_CONTEXT_REPRO_REPOSITORY") {
            for path in [
                "Lib/_py_warnings.py",
                "Lib/_pydatetime.py",
                "Lib/asyncio/sslproto.py",
            ] {
                files.push(tape(
                    &std::fs::read_to_string(std::path::Path::new(&root).join(path)).unwrap(),
                ));
            }
        }
        if let Ok(nodes) = std::env::var("TEMPO_CONTEXT_DUMP_NODES") {
            for node in nodes.split(',').map(|node| node.parse::<u32>().unwrap()) {
                let mut first = 0;
                for (index, file) in files.iter().enumerate() {
                    let last = first + file.transducer_len() as u32;
                    if (first..last).contains(&node) {
                        let local = node - first;
                        let children = file
                            .transducer_children(local)
                            .map(|child| {
                                (
                                    child + first,
                                    file.transducer_kind(child),
                                    file.transducer_field(child),
                                    file.transducer_flags(child),
                                )
                            })
                            .collect::<Vec<_>>();
                        eprintln!(
                            "TEMPO_CONTEXT_NODE global={node} file={index} local={local} span={:?} children={children:?}",
                            file.transducer_span(local)
                        );
                    }
                    first = last;
                }
            }
        }
        let active_programs = std::env::var("TEMPO_CONTEXT_ACTIVE_PROGRAMS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(|index| index.parse::<usize>().unwrap())
                    .collect::<Vec<_>>()
            });
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .enumerate()
                    .map(|(index, program)| {
                        if active_programs
                            .as_ref()
                            .is_some_and(|active| !active.contains(&index))
                        {
                            return Vec::new();
                        }
                        let kinds = program.leading_kinds();
                        (0..file.transducer_len() as u32)
                            .filter(|&node| {
                                kinds
                                    .as_ref()
                                    .is_none_or(|kinds| kinds.contains(&file.transducer_kind(node)))
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch_outcomes(&programs, &files, &anchors)
                .unwrap();
        if std::env::var_os("TEMPO_CONTEXT_DUMP_PROGRAMS").is_some() {
            for index in [115, 278] {
                if let Some(program) = programs.get(index) {
                    for (pc, &word) in program.words.iter().enumerate() {
                        eprintln!(
                            "TEMPO_CONTEXT_PROGRAM program={index} pc={pc} instruction={:?}",
                            crate::code_rule_dsl::transducer::packed::split(word)
                        );
                    }
                }
            }
        }
        assert!(expected.errors.iter().flatten().all(Option::is_none));
        let repetitions = std::env::var("TEMPO_CONTEXT_REPRO_REPEAT")
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1);
        for _ in 0..repetitions {
            let scope = metal_backend::launch::LaunchScope::enter();
            let start = std::time::Instant::now();
            assert_fixture(&programs, &files, &anchors);
            let trace = scope.snapshot();
            assert!(trace.len() <= 600);
            assert!(
                trace
                    .iter()
                    .all(|record| record.encoded && record.submitted && record.completed)
            );
            eprintln!(
                "QUANTIFIED_INJECTION_EVIDENCE {}",
                serde_json::json!({
                    "programs": programs.len(), "files": files.len(), "launches": trace.len(),
                    "fixtureSeconds": start.elapsed().as_secs_f64(),
                    "peakMetalBytes": trace.iter().map(|record| record.peak_application_buffer_bytes).max(),
                })
            );
        }
    }
    #[test]
    fn inputs_checked_capacity_replay_grows_from_measured_wave_demand() {
        let program = Program::new(
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"first".to_vec(), b"second".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let programs = [program];
        let files = [tape(&"# item\n".repeat(40))];
        let anchors = vec![vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_oversized_direct_shapes_execute_on_shared_frontiers_with_raw_cpu_parity() {
        use crate::code_rule_dsl::transducer::packed::Directive;
        let build =
            |words| Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
        let mut programs = Vec::new();
        for count in [9, 130] {
            let mut words = vec![word(CAPTURE, 0, 0); count];
            words.push(word(EMIT, 0, 0));
            programs.push(build(words));
        }
        let mut branches = Vec::new();
        for index in 0..65 {
            branches.extend([
                word(FORK, 0, (index + 1) * 3),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ]);
        }
        branches.extend([word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
        programs.push(build(branches));
        let mut jumps = (1..=257)
            .map(|next| word(GOTO, 0, next))
            .collect::<Vec<_>>();
        jumps.push(word(EMIT, 0, 0));
        programs.push(build(jumps));
        let properties = build(vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)])
            .with_directives(vec![
                Directive::Set {
                    key: 0,
                    value: None
                };
                65
            ])
            .unwrap();
        programs.push(properties);
        let files = [tape("name\n")];
        let anchors = vec![vec![vec![0]; programs.len()]];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][2].len(), 66);
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_bounded_semantic_ranges_preserve_late_failures_limits_and_raw_order() {
        let language = SupportLang::Python.get_ts_language();
        let failing = Program::new(
            vec![
                word(FORK, 0, 4),
                word(TEXT, 0, 0),
                word(RESTORE_CURSOR, 0, 7),
                word(STOP, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"item".to_vec(), b"bad".to_vec()],
            vec![TextPredicate::Equal(1)],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [
            failing,
            Program::compile_tree_sitter_query_pattern("(module (comment)+ @item)", &language)
                .unwrap(),
            Program::compile_tree_sitter_query_pattern("(identifier) @name", &language).unwrap(),
        ];
        let files = [
            tape("good = bad\n# one\n# two\n"),
            tape("left = right\n# three\n# four\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect::<Vec<_>>(); programs.len()])
            .collect::<Vec<_>>();
        {
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_measured_seed_checkpoint_preserves_semantics_and_releases_execution_storage() {
        let programs = [Program::compile_tree_sitter_query_pattern(
            "(module (comment)+ @item)",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let files = [tape("# one\n# two\n# three\n")];
        let anchors = vec![vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_streamed_pages_publish_only_complete_files() {
        let program = Program::compile_tree_sitter_query_pattern(
            "(module (expression_statement (assignment left: (identifier) @name)))",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let programs = [program];
        let files = [
            tape("a = b\nc = d\n"),
            tape("e = f\n"),
            tape("g = h\ni = j\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect()])
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_typed_pages_merge_direct_results_and_match_cpu() {
        let structural = Program::new(
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"first".to_vec(), b"second".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let direct = Program::compile_tree_sitter_query_pattern(
            "(identifier) @name",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let programs = [structural, direct];
        let files = [
            tape("call(one, two)\nother(three)\n"),
            tape("first = second\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        {
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_compact_shared_values_restore_structural_context_after_history_compaction() {
        let pattern = "a".repeat(80);
        let programs = [false, true].map(|lua| {
            Program::new(
                vec![
                    word(SAVE_CURSOR, 0, 7),
                    word(REPEAT_BEGIN, 0, 0),
                    word(CHILD, 1, 0),
                    word(TEXT, 0, 0),
                    word(REPEAT_BUMP, 0, 0),
                    word(REPEAT_END, 0, 0),
                    word(RESTORE_CURSOR, 0, 7),
                    word(CAPTURE, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![pattern.as_bytes().to_vec(), b"root".to_vec()],
                vec![if lua {
                    TextPredicate::LuaPattern {
                        source: 0,
                        compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                    }
                } else {
                    TextPredicate::Regex {
                        source: 0,
                        compiled: regex::Regex::new(&pattern).unwrap(),
                    }
                }],
                vec![1],
                vec![],
            )
            .unwrap()
        });
        let files = [tape(&format!("{pattern}\n"))];
        let anchors = [vec![vec![0]; 2]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_bounded_terminal_expansions_keep_four_children_in_registers_and_route_larger_sets() {
        let language = SupportLang::Python.get_ts_language();
        let queries = [
            r#"((argument_list (identifier) @argument) (#set! role "argument"))"#,
            r#"(argument_list (identifier) @argument . "," @separator)"#,
        ];
        let programs = queries
            .iter()
            .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        for count in 0..=6 {
            let arguments = (0..count)
                .map(|index| format!("arg{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let files = [tape(&format!("call({arguments})\n"))];
            let anchors = [programs
                .iter()
                .map(|_| (0..files[0].transducer_len() as u32).collect())
                .collect()];
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_staged_unique_navigation_failure_retains_its_branch_history() {
        let programs = [Program::new(
            vec![
                word(CHILD, 1, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(STOP, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()];
        let files = [tape("first(one)\n")];
        let anchors = [vec![vec![0]]];
        assert_eq!(
            execute_batch(&programs, &files, Some(&anchors)).unwrap_err(),
            Error::InvalidOperand(1)
        );
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_staged_unique_navigation_emits_in_registers_before_candidate_comparison() {
        let programs = [Program::compile_tree_sitter_query_pattern(
            "(call function: (identifier) @name)",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let files = [tape("first(one)\nsecond(two)\n")];
        let anchors = [vec![(0..files[0].transducer_len() as u32).collect()]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_bounded_paths_keep_unique_children_in_registers_and_branching_children_in_frontiers()
    {
        let language = SupportLang::Python.get_ts_language();
        let patterns = [
            "(_ (block) @inner) @outer",
            "(call function: (identifier) @name)",
            "(function_definition name: (identifier) @name)",
            "(call function: [(identifier) (attribute attribute: (identifier))] @name)",
            "(call function: [(identifier) @name (identifier) @duplicate])",
            r#"([(call function: (identifier) @name) (call function: (identifier) @name arguments: (argument_list) @name)] (#match? @name "^c"))"#,
            "((call function: [(identifier) (attribute attribute: (identifier))] @name) (#set! role \"callee\"))",
        ];
        let mut programs = patterns
            .iter()
            .map(|pattern| Program::compile_tree_sitter_query_pattern(pattern, &language).unwrap())
            .collect::<Vec<_>>();
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
        let files = [
            tape("def first():\n    call(one, two)\ndef second():\n    pass\n"),
            tape("if first:\n    pass\nelse:\n    call(one)\n"),
        ];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            crate::code_rule_dsl::transducer::packed::execute_with_backend(
                crate::code_rule_dsl::transducer::packed::ExecutionBackend::CpuJit,
                &programs,
                &files,
                &anchors
            )
            .unwrap(),
            expected
        );
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_failure_order_follows_dfs_instead_of_physical_append_order() {
        let program = Program::new(
            vec![
                word(FORK, 0, 4),
                word(GOTO, 0, 6),
                word(STOP, 0, 0),
                word(STOP, 0, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(STOP, 0, 0),
                word(GOTO, 0, 8),
                word(STOP, 0, 0),
                word(RESTORE_CURSOR, 0, 1),
                word(STOP, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [program];
        let files = [tape("one = two\n")];
        let anchors = [vec![vec![0]]];
        assert!(matches!(
            execute_batch(&programs, &files, Some(&anchors)),
            Err(Error::InvalidOperand(8))
        ));
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_direct_grid_pages_match_cpu_at_every_small_capacity_and_suppress_failed_evaluations()
    {
        let language = SupportLang::Python.get_ts_language();
        let first =
            Program::compile_tree_sitter_query_pattern("(identifier) @name", &language).unwrap();
        let second =
            Program::compile_tree_sitter_query_pattern("(integer) @number", &language).unwrap();
        let programs = vec![first, second];
        let files = vec![tape("left = right + 1\n"), tape("other = value + 2\n")];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect::<Vec<_>>(); programs.len()])
            .collect::<Vec<_>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_direct_pages_keep_multi_result_tuples_whole_and_preserve_capture_order() {
        let program = Program::new(
            vec![
                word(FORK, 0, 3),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(CAPTURE, 0, 2),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
            vec![],
            vec![0, 1, 2],
            vec![],
        )
        .unwrap();
        let files = vec![tape("call(one, two)\n")];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        assert_fixture(std::slice::from_ref(&program), &files, &anchors);
    }
    #[test]
    fn inputs_adaptive_pages_retry_from_the_last_published_checkpoint_without_duplicates() {
        let call = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("call", true);
        let program = Program::new(
            vec![
                word(FORK, 0, 3),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(KIND, 0, call),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"all".to_vec(), b"call".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let programs = [program];
        for prefix in [3, 600] {
            let files = [tape(&format!("{}call(value)\n", "name\n".repeat(prefix)))];
            let anchors = vec![vec![
                (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
            ]];
            assert_fixture(&programs, &files, &anchors);
        }
    }
}

mod retained_grid_frontier_runtime {
    use super::*;
    fn program(words: Vec<u32>) -> Program {
        Program::new(
            words,
            vec![b"root".to_vec(), b"child".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
    }
    fn assert_stage_parity(programs: &[Program], files: &[TapeDoc]) {
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect())
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_fixture(programs, files, &anchors);
    }
    #[test]
    fn inputs_sparse_output_groups_preserve_raw_candidate_offsets_without_history_growth() {
        use crate::code_rule_dsl::transducer::packed::FORK;
        let mut words = Vec::new();
        for branch in 0..65 {
            words.extend([
                word(FORK, 0, ((branch + 1) * 3) as u16),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ]);
        }
        words.extend([word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
        let programs =
            [Program::new(words, vec![b"item".to_vec()], vec![], vec![0], vec![]).unwrap()];
        let files = [tape("name\n")];
        let anchors = [vec![vec![0]]];
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        assert_eq!(expected[0][0].len(), 66);
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_linear_strip_units_preserve_full_input_find_costs_and_work_limits() {
        use crate::code_rule_dsl::transducer::packed::{Directive, value::ByteNfa};
        for (pattern, source) in [
            ("^n", "name\n"),
            ("^", "éx\n"),
            ("^$", ""),
            ("^é", "éx\n"),
            ("^[a-z]", "name\n"),
            ("^ab", "abc\n"),
            ("^z", "name\n"),
            ("^(n)", "name\n"),
        ] {
            let nfa = ByteNfa::compile(pattern).unwrap();
            let regex = regex::Regex::new(pattern).unwrap();
            let mut search = 0;
            let mut work = 0;
            while search <= source.len() {
                let (found, cost) = nfa.test_find_work_count_from(source.as_bytes(), search);
                let expected = regex
                    .find_at(source, search)
                    .map(|matched| [matched.start() as u32, matched.end() as u32]);
                assert_eq!(
                    found, expected,
                    "CPU closure oracle {pattern} start={search}"
                );
                work += cost;
                let Some([first, end]) = found else {
                    break;
                };
                search = end as usize + usize::from(first == end);
                while search < source.len() && source.as_bytes()[search] & 192 == 128 {
                    search += 1;
                }
            }
            let programs = [Program::new(
                vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![Directive::Strip {
                capture: 0,
                source: 1,
                compiled: regex,
            }])
            .unwrap()];
            let files = [tape(source)];
            let anchors = [vec![vec![0]]];
            for limit in [work - 1, work, work + 1] {
                assert_strip_budget(pattern, source, limit);
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_non_strip_output_preparation_avoids_strip_pools_and_matches_cpu() {
        use crate::code_rule_dsl::transducer::packed::Directive;
        let programs = [Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"text".to_vec(), b"kind".to_vec(), b"value".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_directives(vec![
            Directive::Set {
                key: 1,
                value: Some(2),
            },
            Directive::SetCapture {
                capture: 0,
                key: 1,
                value: None,
            },
            Directive::Downcase { capture: 0 },
            Directive::SetLanguageFromMimeType { capture: 0 },
        ])
        .unwrap()];
        let files = [tape("TEXT/PLAIN\n")];
        let anchors = [vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_transform_index_barrier_drains_empty_and_exact_boundary_producers() {
        use crate::code_rule_dsl::transducer::packed::Directive;
        let mut words = vec![word(FORK, 0, 45)];
        words.extend(std::iter::repeat_n(word(CAPTURE, 0, 0), 43));
        words.extend([word(EMIT, 0, 0), word(CAPTURE, 0, 1), word(EMIT, 0, 0)]);
        let programs = [Program::new(
            words,
            vec![b"text".to_vec(), b"other".to_vec(), b"a".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_directives(vec![
            Directive::Strip {
                capture: 0,
                source: 2,
                compiled: regex::Regex::new("a").unwrap(),
            },
            Directive::Strip {
                capture: 0,
                source: 2,
                compiled: regex::Regex::new("a").unwrap(),
            },
        ])
        .unwrap()];
        let files = [tape("aZa\n")];
        let anchors = [vec![vec![0]]];
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        assert_eq!(expected[0][0].len(), 2);
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_transform_pair_cursors_resume_and_survive_history_compaction() {
        use crate::code_rule_dsl::transducer::packed::Directive;
        let mut words = vec![word(CAPTURE, 0, 0); 130];
        words.push(word(EMIT, 0, 0));
        let base = Program::new(
            words,
            vec![
                b"text".to_vec(),
                b"a".to_vec(),
                b"# ".to_vec(),
                b"kind".to_vec(),
                b"value".to_vec(),
                b"_".to_vec(),
            ],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [base
            .with_directives(vec![
                Directive::Downcase { capture: 0 },
                Directive::Strip {
                    capture: 0,
                    source: 1,
                    compiled: regex::Regex::new("a").unwrap(),
                },
                Directive::Strip {
                    capture: 0,
                    source: 2,
                    compiled: regex::Regex::new("# ").unwrap(),
                },
                Directive::Strip {
                    capture: 0,
                    source: 5,
                    compiled: regex::Regex::new("_").unwrap(),
                },
                Directive::SetCapture {
                    capture: 0,
                    key: 3,
                    value: Some(4),
                },
            ])
            .unwrap()];
        let files = [tape("# AaZa\n")];
        let anchors = [vec![vec![0]]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_parallel_candidate_mapping_covers_dense_signatures_and_partial_blocks() {
        let programs = [
            program(vec![
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ]),
            program(vec![
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 1),
            ]),
        ];
        let source = (0..513).map(|i| format!("name_{i}\n")).collect::<String>();
        let files = [tape(&source)];
        let anchors = [vec![vec![0]; programs.len()]];
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_compact_stage_waves_match_cpu_navigation_and_control_flow() {
        let programs = vec![
            program(vec![
                word(CAPTURE, 0, 0),
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(CAPTURE, 0, 1),
                word(RESTORE_CURSOR, 0, 0),
                word(CHILD, 1, 0),
                word(PARENT, 0, 0),
                word(EMIT, 0, 0),
            ]),
            program(vec![
                word(DESCEND, 1, 0),
                word(CAPTURE, 0, 1),
                word(NEXT, 1, 0),
                word(EMIT, 0, 1),
            ]),
            program(vec![
                word(FOLLOWING, 1, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 1),
            ]),
            program(vec![
                word(FORK, 0, 3),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(CHILD, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 1),
            ]),
            program(vec![
                word(REPEAT_BEGIN, 0, 0),
                word(FORK, 0, 4),
                word(REPEAT_END, 0, 0),
                word(EMIT, 0, 0),
                word(REPEAT_END, 0, 0),
                word(EMIT, 0, 0),
            ]),
        ];
        assert_stage_parity(
            &programs,
            &[
                tape("call(one, two)\nother(three)\n"),
                tape("first = second\n"),
            ],
        );
    }
    #[test]
    fn inputs_relation_expansion_seeds_are_bounded_and_continuations_preserve_raw_order() {
        let source = (0..130)
            .map(|index| format!("item_{index} = (left + right)\n"))
            .collect::<String>();
        let files = [tape(&source)];
        for opcode in [CHILD, crate::code_rule_dsl::transducer::packed::DESCEND, FOLLOWING] {
            for named in [0, 1] {
                let programs = [program(vec![
                    word(opcode, named, 0),
                    word(CAPTURE, 0, 1),
                    word(EMIT, 0, 0),
                ])];
                let root = if opcode == FOLLOWING {
                    files[0].transducer_children(0).next().unwrap()
                } else {
                    0
                };
                let anchors = [vec![vec![root]]];
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_long_literal_predicates_advance_in_shared_bounded_units() {
        let mut programs = Vec::new();
        let mut source = String::new();
        for length in [65, 1025] {
            let literal = "a".repeat(length);
            source.push_str(&format!(
                "{literal}\n{literal}_suffix\n{}b\n",
                "a".repeat(length - 1)
            ));
            for prefix in [false, true] {
                programs.push(
                    Program::new(
                        vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                        vec![
                            b"root".to_vec(),
                            b"name".to_vec(),
                            literal.as_bytes().to_vec(),
                        ],
                        vec![if prefix {
                            crate::code_rule_dsl::transducer::packed::TextPredicate::Prefix(2)
                        } else {
                            crate::code_rule_dsl::transducer::packed::TextPredicate::Equal(2)
                        }],
                        vec![1],
                        vec![],
                    )
                    .unwrap(),
                );
            }
            for predicate in ["eq?", "not-eq?"] {
                programs.push(
                    Program::compile_tree_sitter_query_pattern(
                        &format!("((identifier) @name (#{predicate} @name \"{literal}\"))"),
                        &SupportLang::Python.get_ts_language(),
                    )
                    .unwrap(),
                );
            }
        }
        let files = [tape(&source), tape(&format!("# shifted source\n{source}"))];
        let structural = programs
            .iter()
            .filter(|program| program.capture_programs.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        assert_stage_parity(&structural, &files);
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_repeated_disjoint_alternatives_share_a_bounded_capture_tail() {
        let source = (0..130)
            .map(|i| {
                if i % 2 == 0 {
                    format!("import item_{i}\n# gap\n")
                } else {
                    format!("from module import item_{i}\n")
                }
            })
            .collect::<String>();
        let files = [tape(&source)];
        for (query, root) in [
            (
                "(module [(import_statement) (import_from_statement)]+ @fold)",
                0,
            ),
            (
                "[(import_statement) (import_from_statement)]+ @fold",
                files[0].transducer_children(0).next().unwrap(),
            ),
        ] {
            let programs = [Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap()];
            let anchors = [vec![vec![root]]];
            assert_fixture(&programs, &files, &anchors);
        }
    }
    #[test]
    fn inputs_absent_field_uses_shared_facts_for_wide_and_file_local_inputs() {
        let wide = (0..130)
            .map(|i| format!("item_{i} = value\n"))
            .collect::<String>();
        let files = [tape(&wide), tape("def named(arg):\n    return arg\n")];
        let language = SupportLang::Python.get_ts_language();
        let name = language.field_id_for_name("name").unwrap().get();
        let body = language.field_id_for_name("body").unwrap().get();
        let programs = [name, body, u16::MAX]
            .into_iter()
            .map(|field| {
                program(vec![
                    word(crate::code_rule_dsl::transducer::packed::ABSENT_FIELD, 0, field),
                    word(CAPTURE, 0, 0),
                    word(EMIT, 0, 0),
                ])
            })
            .collect::<Vec<_>>();
        assert_stage_parity(&programs, &files);
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_endpoint_guards_preserve_named_and_anonymous_sibling_boundaries() {
        let files = [tape("call(left, right, third)\n# comment\n")];
        let programs = [
            crate::code_rule_dsl::transducer::packed::FIRST,
            crate::code_rule_dsl::transducer::packed::LAST,
        ]
        .into_iter()
        .flat_map(|opcode| {
            [0, 1].into_iter().map(move |named| {
                program(vec![
                    word(opcode, named, 0),
                    word(CAPTURE, 0, 0),
                    word(EMIT, 0, 0),
                ])
            })
        })
        .collect::<Vec<_>>();
        assert_stage_parity(&programs, &files);
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_fixture(&programs, &files, &anchors);
    }
}

mod retained_program_backend {
    use super::*;
    #[test]
    fn inputs_metal_workspace_reuse_preserves_quantified_results() {
        let source = "# one\n# two\nvalue = 1\n";
        let files = vec![tape(source)];
        let program = Program::compile_tree_sitter_query_pattern(
            "(module (comment)+ @comments)",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let roots = (0..files[0].transducer_len() as u32).collect::<Vec<_>>();
        let anchors = vec![vec![roots]];
        assert_fixture(std::slice::from_ref(&program), &files, &anchors);
    }
    #[test]
    fn inputs_structural_and_mixed_batches_use_only_the_grid_scheduler() {
        let direct = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let structural = Program::new(
            vec![word(CHILD, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let files = vec![tape("call(one)\n")];
        for programs in [vec![structural.clone()], vec![direct, structural]] {
            let anchors = vec![vec![vec![0]; programs.len()]];
            {
                {
                    assert_fixture(&programs, &files, &anchors);
                }
            }
        }
    }
    #[test]
    fn inputs_batch_executor_runs_linear_structural_program_in_one_resident_grid() {
        let grammar = SupportLang::Python.get_ts_language();
        let kind = grammar.id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![word(KIND, 0, kind), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let files = vec![tape("name\n")];
        let anchors = vec![vec![(0..files[0].transducer_len() as u32).collect()]];

        assert_fixture(&[program], &files, &anchors);
    }
    #[test]
    fn inputs_batch_executor_runs_child_navigation_in_one_resident_grid() {
        let program = Program::new(
            vec![
                word(CAPTURE, 0, 0),
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let files = vec![tape("call(one, two)\n")];
        let anchors = vec![vec![(0..files[0].transducer_len() as u32).collect()]];

        assert_fixture(&[program], &files, &anchors);
    }
    #[test]
    fn inputs_batch_executor_runs_equal_and_prefix_value_programs_internally() {
        let programs = [TextPredicate::Equal(0), TextPredicate::Prefix(0)]
            .into_iter()
            .map(|predicate| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                    vec![b"name".to_vec(), b"match".to_vec()],
                    vec![predicate],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = vec![tape("name\n")];
        let anchors = vec![
            programs
                .iter()
                .map(|_| (0..files[0].transducer_len() as u32).collect())
                .collect(),
        ];

        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_batch_executor_runs_regex_nfa_inside_metal() {
        let program = Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec(), b"output".to_vec()],
            vec![TextPredicate::Regex {
                source: 0,
                compiled: regex::Regex::new("match").unwrap(),
            }],
            vec![1],
            vec![],
        )
        .unwrap();
        let files = vec![tape("match\n")];
        let anchors = vec![vec![(0..files[0].transducer_len() as u32).collect()]];

        assert_fixture(&[program], &files, &anchors);
    }
    #[test]
    fn inputs_batch_executor_matches_unicode_word_boundaries_inside_metal() {
        let patterns = [
            r"\bαβ\b",
            r"\Bα",
            r"\b{start}α",
            r"β\b{end}",
            r"\b{start-half}α",
            r"β\b{end-half}",
            r"(?-u:\b)α",
        ];
        let programs = patterns
            .iter()
            .map(|pattern| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.as_bytes().to_vec(), b"output".to_vec()],
                    vec![TextPredicate::Regex {
                        source: 0,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = vec![tape("αβ = 1\n")];
        let identifier = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("identifier", true);
        let root = (0..files[0].transducer_len() as u32)
            .find(|node| files[0].transducer_kind(*node) == identifier)
            .unwrap();
        let anchors = vec![vec![vec![root]; programs.len()]];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert!(!expected[0][0].is_empty());
        assert!(expected[0][1].is_empty());
        assert!(expected[0][6].is_empty());

        assert_fixture(&programs, &files, &anchors);
    }
    #[test]
    fn inputs_batch_executor_runs_lua_program_inside_metal() {
        let program = Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            vec![b"%a+".to_vec(), b"output".to_vec()],
            vec![TextPredicate::LuaPattern {
                source: 0,
                compiled: LuaPattern::parse(b"%a+").unwrap(),
            }],
            vec![1],
            vec![],
        )
        .unwrap();
        let files = vec![tape("name\n")];
        let anchors = vec![vec![(0..files[0].transducer_len() as u32).collect()]];

        assert_fixture(&[program], &files, &anchors);
    }
    #[test]
    fn inputs_batch_executor_differentiates_lua_language_inside_metal() {
        let cases = [
            ("abc123\n", b"%a+%d+".as_slice()),
            ("(name)\n", b"%b()".as_slice()),
            ("nana\n", b"(%a+)%1".as_slice()),
            ("name\n", b"%f[%a]name".as_slice()),
            ("name\n", b"()name".as_slice()),
            ("aaaaab\n", b"a.-b".as_slice()),
            ("b\n", b"a?b".as_slice()),
            ("aaab\n", b"a*b".as_slice()),
            ("aaab\n", b"a+b".as_slice()),
        ];
        let programs = cases
            .iter()
            .map(|(_, pattern)| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                    vec![pattern.to_vec(), b"output".to_vec()],
                    vec![TextPredicate::LuaPattern {
                        source: 0,
                        compiled: LuaPattern::parse(pattern).unwrap(),
                    }],
                    vec![1],
                    vec![],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = cases
            .iter()
            .map(|(source, _)| tape(source))
            .collect::<Vec<_>>();
        let anchors = (0..cases.len())
            .map(|file| {
                (0..cases.len())
                    .map(|program| if file == program { vec![0] } else { vec![] })
                    .collect()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        for (index, outputs) in expected.iter().enumerate() {
            assert!(!outputs[index].is_empty(), "case {index}");
        }
        {
            {
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_batch_executor_runs_strip_output_program_inside_metal() {
        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"^n".to_vec(), b"match".to_vec()],
            vec![],
            vec![1],
            vec![],
        )
        .unwrap()
        .with_directives(vec![Directive::Strip {
            capture: 0,
            source: 0,
            compiled: regex::Regex::new("^n").unwrap(),
        }])
        .unwrap();
        let files = vec![tape("name\n")];
        let anchors = vec![vec![(0..files[0].transducer_len() as u32).collect()]];

        assert_fixture(&[program], &files, &anchors);
    }
}

mod retained_grid_history_runtime {
    use super::*;
    #[test]
    fn inputs_lua_priority_barrier_suspends_before_charging_the_exact_first_prefix() {
        use crate::code_rule_dsl::transducer::packed::{EMIT, LuaPattern, TEXT, TextPredicate, word};
        let text = "identifier\n";
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(text, SupportLang::Python);
        let image = TapeImage::encode(native.tree.root_node(), &grammar, text, &|| false).unwrap();
        let file = TapeDoc::restore(Arc::new(text.to_owned()), grammar, &image, &|| false).unwrap();
        let lua = LuaPattern::parse(b"Z").unwrap();
        let program = Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            vec![b"Z".to_vec(), b"text".to_vec()],
            vec![TextPredicate::LuaPattern {
                source: 0,
                compiled: lua,
            }],
            vec![1],
            vec![],
        )
        .unwrap();
        let crate::code_rule_dsl::transducer::packed::value::ValueProgram::Lua(lua) =
            &program.value_programs[0]
        else {
            unreachable!()
        };
        let (matched, _) = lua.test_work_count(text.as_bytes());
        assert!(!matched.unwrap());
        assert_fixture(&[program], &[file], &[vec![vec![0]]]);
    }
    #[test]
    fn inputs_captured_text_argument_cursors_survive_collection_value_and_advance_compaction() {
        use crate::code_rule_dsl::transducer::packed::{
            ASSERT, CAPTURE, CapturePredicate, EMIT, LuaPattern, REPEAT_BEGIN, REPEAT_BUMP,
            REPEAT_END, RESTORE_CURSOR, SAVE_CURSOR, TextPredicate, word,
        };
        let source = format!("{}Z", "a".repeat(129));
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(&source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
        let file = TapeDoc::restore(Arc::new(source.clone()), grammar, &image, &|| false).unwrap();
        let regex = regex::Regex::new("^a").unwrap();
        for lua in [false, true] {
            for scalar in [false, true] {
                let mut words = vec![word(SAVE_CURSOR, 0, 7), word(REPEAT_BEGIN, 0, 0)];
                for _ in 0..65 {
                    words.extend([word(CAPTURE, 0, 0), word(CAPTURE, 0, 1)]);
                }
                words.extend([
                    word(ASSERT, 0, 0),
                    word(REPEAT_BUMP, 0, 0),
                    word(REPEAT_END, 0, 0),
                    word(RESTORE_CURSOR, 0, 7),
                    word(CAPTURE, 0, 2),
                    word(EMIT, 0, 2),
                ]);
                let pattern = if lua && scalar { "Z" } else { "^a" };
                let predicate = if lua {
                    TextPredicate::LuaPattern {
                        source: 0,
                        compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                    }
                } else {
                    TextPredicate::Regex {
                        source: 0,
                        compiled: regex.clone(),
                    }
                };
                let program = Program::new_with_capture_predicates(
                    words,
                    vec![
                        pattern.as_bytes().to_vec(),
                        b"argument".to_vec(),
                        b"other".to_vec(),
                        b"root".to_vec(),
                    ],
                    vec![predicate],
                    vec![CapturePredicate::Text {
                        capture: 0,
                        predicate: 0,
                        negated: false,
                        any: false,
                    }],
                    vec![1, 2, 3],
                    vec![],
                )
                .unwrap();
                let work = match &program.value_programs[0] {
                    crate::code_rule_dsl::transducer::packed::value::ValueProgram::Regex(value) => {
                        value.test_work_count(source.as_bytes()).1
                    }
                    crate::code_rule_dsl::transducer::packed::value::ValueProgram::Lua(value) => {
                        value.test_work_count(source.as_bytes()).1
                    }
                    _ => unreachable!(),
                };
                let programs = [program];
                let files = [file.clone()];
                let anchors = [vec![vec![0]]];
                let expected =
                    crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                        .unwrap();
                assert_eq!(expected[0][0].len(), 1);
                assert_eq!(expected[0][0][0].captures.len(), 131);
                assert_value_budget(&programs[0], &source, work, false);
                assert_fixture(&programs, &files, &anchors);
            }
        }
    }
    #[test]
    fn inputs_capture_predicate_collection_and_pair_lists_survive_forced_compaction() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CAPTURE, CapturePredicate, EMIT, word};
        let source = format!("{}Z", "a".repeat(129));
        let literal = source.as_bytes().to_vec();
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(&source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
        let file = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
        let mut words = (0..140)
            .map(|index| word(CAPTURE, 0, index % 2))
            .collect::<Vec<_>>();
        words.extend([word(ASSERT, 0, 0), word(EMIT, 0, 0)]);
        let files = [file];
        for predicate in [
            CapturePredicate::Equal {
                left: 0,
                right: 1,
                negated: false,
                any: false,
            },
            CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![2],
                negated: false,
                any: false,
            },
            CapturePredicate::Contains {
                capture: 0,
                strings: vec![2],
                negated: false,
                any: false,
            },
            CapturePredicate::Relation {
                capture: 0,
                kinds: vec![0],
                ancestor: true,
                negated: true,
            },
        ] {
            let program = Program::new_with_capture_predicates(
                words.clone(),
                vec![b"left".to_vec(), b"right".to_vec(), literal.clone()],
                vec![],
                vec![predicate],
                vec![0, 1],
                vec![],
            )
            .unwrap();
            let programs = [program];
            let anchors = [vec![vec![0]]];
            let expected =
                crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                    .unwrap();
            assert_eq!(expected[0][0].len(), 1);
            assert_fixture(&programs, &files, &anchors);
        }
    }
}

// Calls the anchor-dependent differential above; ported with it.
#[cfg(any())]
#[test]
#[ignore = "all pinned patterns through the successor-view operator against the independent CPU interpreter"]
fn pinned_corpus_successor_view_differential() {
    let _views = metal_dynamic_execution::ViewConformanceScope::enter();
    pinned_corpus_optional_body_and_injection_differential();
}

#[test]
fn long_interpolated_query_preserves_every_capture_and_property_within_the_original_evaluation_limits()
 {
    long_interpolated_query_oracles(false, true);
}

#[test]
#[ignore = "release CPU baseline for complete long captured-query execution"]
fn long_interpolated_query_complete_cpu_reference() {
    long_interpolated_query_oracles(true, false);
}

fn long_interpolated_query_oracles(complete_cpu: bool, metal: bool) {
    use tree_sitter::StreamingIterator;
    let query = r#"((call function: (attribute object: (identifier) @_re)
      arguments: (argument_list (string (string_start)
        [(string_content) @injection.content (interpolation)]+ (string_end))))
      (#eq? @_re "re") (#set! injection.language "regex"))"#;
    let language = SupportLang::Python.get_ts_language();
    let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
    let label = |name: &[u8]| {
        u16::try_from(
            program
                .labels
                .iter()
                .position(|&i| program.strings[i as usize] == name)
                .unwrap(),
        )
        .unwrap()
    };
    let string = |name: &[u8]| {
        u16::try_from(program.strings.iter().position(|s| s == name).unwrap()).unwrap()
    };
    for parts in [1usize, 8, 350] {
        let source = format!(
            "import re\npattern = re.compile(f\"a{}\")\n",
            "{value}a".repeat(parts)
        );
        let files = [tape(&source)];
        let call_kind = language.id_for_node_kind("call", true);
        let calls = (0..files[0].transducer_len() as u32)
            .filter(|&node| files[0].transducer_kind(node) == call_kind)
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        let re_start = source.find("re.compile").unwrap();
        let mut expected_spans = vec![(label(b"_re"), re_start..re_start + 2)];
        let content_start = source.find("f\"a").unwrap() + 2;
        for part in 0..=parts {
            expected_spans.push((
                label(b"injection.content"),
                content_start + part * 8..content_start + part * 8 + 1,
            ));
        }
        let captures = expected_spans
            .iter()
            .map(|(label, span)| {
                let nodes = (0..files[0].transducer_len() as u32)
                    .filter(|&node| files[0].transducer_span(node) == *span)
                    .collect::<Vec<_>>();
                assert_eq!(nodes.len(), 1, "capture span {span:?}");
                Capture {
                    label: *label,
                    node: nodes[0],
                    span: span.clone(),
                    text: None,
                    properties: vec![],
                }
            })
            .collect::<Vec<_>>();
        let expected = vec![vec![vec![Output {
            task_ordinal: 0,
            local_ordinal: 0,
            start: calls[0],
            label: label(b"tree-sitter-query"),
            captures,
            properties: vec![(string(b"injection.language"), Some(string(b"regex")))],
            dynamic_properties: vec![],
        }]]];
        // Native Tree-sitter supplies an independent complete capture-order oracle
        // at the long boundary without hashing every interpreter history.
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let native_query = tree_sitter::Query::new(&language, query).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&native_query, tree.root_node(), source.as_bytes());
        let mut native = vec![];
        while let Some(found) = matches.next() {
            native.push(
                found
                    .captures()
                    .iter()
                    .map(|c| {
                        (
                            native_query.capture_names()[c.index as usize].to_owned(),
                            c.node.start_byte()..c.node.end_byte(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(
            native,
            vec![
                expected_spans
                    .iter()
                    .map(|(capture, span)| (
                        String::from_utf8(
                            program.strings[program.labels[*capture as usize] as usize].clone()
                        )
                        .unwrap(),
                        span.clone()
                    ))
                    .collect::<Vec<_>>()
            ]
        );
        drop(matches);
        assert!(!cursor.did_exceed_match_limit());
        let anchors = [vec![calls]];
        if parts <= 8 || complete_cpu {
            let started = std::time::Instant::now();
            let interpreted = execute_batch(std::slice::from_ref(&program), &files, Some(&anchors));
            eprintln!(
                "LONG_INTERPOLATED_CPU_REFERENCE parts={parts} seconds={} error={:?}",
                started.elapsed().as_secs_f64(),
                interpreted.as_ref().err()
            );
            assert_eq!(interpreted.unwrap(), expected);
            if complete_cpu {
                continue;
            }
            assert_eq!(
                execute_with_backend(
                    ExecutionBackend::CpuJit,
                    std::slice::from_ref(&program),
                    &files,
                    &anchors
                )
                .unwrap(),
                expected
            );
        }
        for forced_views in [false, true]
            .into_iter()
            .filter(|force| metal && (!force || parts <= 8))
        {
            let _views = forced_views.then(metal_dynamic_execution::ViewConformanceScope::enter);
            let scope = metal_backend::launch::LaunchScope::enter();
            let started = std::time::Instant::now();
            let actual = execute_with_backend(
                ExecutionBackend::Metal,
                std::slice::from_ref(&program),
                &files,
                &anchors,
            );
            let trace = scope.snapshot();
            eprintln!(
                "LONG_INTERPOLATED_QUERY_EVIDENCE {}",
                serde_json::json!({"forcedViews":forced_views,"parts":parts,"nodes":files[0].transducer_len(),"captures":expected[0][0][0].captures.len(),"attempts":trace.len(),"encoded":trace.iter().filter(|r|r.encoded).count(),"completed":trace.iter().filter(|r|r.completed).count(),"seconds":started.elapsed().as_secs_f64(),"error":format!("{:?}",actual.as_ref().err()),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),"lastKernels":trace.iter().rev().take(12).map(|r|&r.kernel).collect::<Vec<_>>()})
            );
            assert_eq!(
                actual.expect(
                    "the complete interpolated query must fit the unchanged evaluation limits"
                ),
                expected
            );
            assert!(!trace.is_empty() && trace.len() <= 600);
            assert!(trace.iter().all(|r| r.encoded
                && r.submitted
                && r.completed
                && r.peak_application_buffer_bytes <= metal_backend::METAL_BUFFER_LIMIT));
            if forced_views {
                assert!(trace.iter().any(|r| r.kernel == "expanded_publish"));
            }
        }
    }
}
