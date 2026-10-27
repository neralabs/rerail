use super::capture_run::Limits;
use super::*;
use crate::policy_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::{LanguageExt, SupportLang};

fn tape(source: &str) -> TapeDoc {
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}
pub(super) fn program() -> Program {
    let lang = SupportLang::Python.get_ts_language();
    let kind = |name| lang.id_for_node_kind(name, true);
    Program::new(
        vec![
            word(SAVE_CURSOR, 0, 0),
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(FOLLOWING, NAMED, 0),
            word(FORK, 0, 8),
            word(KIND, 0, kind("identifier")),
            word(CAPTURE, 0, 1),
            word(GOTO, 0, 9),
            word(KIND, 0, kind("integer")),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, 0, 19),
            word(NEXT, NAMED, 0),
            word(FORK, 0, 16),
            word(KIND, 0, kind("identifier")),
            word(CAPTURE, 0, 1),
            word(GOTO, 0, 17),
            word(KIND, 0, kind("integer")),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 10),
            word(REPEAT_END, 0, 0),
            word(FOLLOWING, NAMED, 0),
            word(KIND, 0, kind("string")),
            word(RESTORE_CURSOR, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"prefix".to_vec(), b"body".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
}
fn root(tape: &TapeDoc) -> u32 {
    let roots = (0..tape.transducer_len() as u32)
        .filter(|&node| tape.transducer_text(node) == b"(")
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    roots[0]
}
fn capture(tape: &TapeDoc, label: u16, text: &[u8]) -> Capture {
    let nodes = (0..tape.transducer_len() as u32)
        .filter(|&node| tape.transducer_text(node) == text)
        .collect::<Vec<_>>();
    assert_eq!(nodes.len(), 1);
    Capture {
        label,
        node: nodes[0],
        span: tape.transducer_span(nodes[0]),
        text: None,
        properties: vec![],
    }
}
#[test]
fn capture_runs_execute_zero_capture_joins_and_preserve_complete_results_and_raw_ordinal_gaps() {
    let doc = tape("f(a,1,b,\"sep\",c,2,\"end\")\n");
    let p = program();
    let root = root(&doc);
    let expected = [(0, vec![b"a".as_slice(), b"b"]), (4, vec![b"c".as_slice()])]
        .into_iter()
        .map(|(ordinal, names)| {
            let mut captures = vec![capture(&doc, 0, b"(")];
            captures.extend(names.into_iter().map(|name| capture(&doc, 1, name)));
            Output {
                task_ordinal: 0,
                local_ordinal: ordinal,
                start: root,
                label: 0,
                captures,
                properties: vec![],
                dynamic_properties: vec![],
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        p.execute_roots(&doc, [root]).unwrap(),
        expected,
        "compiled CPU oracle must establish the fixture first"
    );
    let compiled = super::capture_run::prepare(&p).unwrap();
    let actual = compiled.execute(&doc, root, Limits::default()).unwrap();
    assert_eq!(actual.outputs, expected);
    eprintln!("CAPTURE_RUN_REPORT {:?}", actual.report);
}

thread_local! {
    static TASK_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_WORK_PER_TASK) };
}
pub(super) fn task_work_limit() -> usize {
    TASK_LIMIT.with(std::cell::Cell::get)
}
pub(super) struct TaskLimit(usize);
impl TaskLimit {
    pub(super) fn enter(limit: usize) -> Self {
        Self(TASK_LIMIT.with(|value| value.replace(limit)))
    }
}
impl Drop for TaskLimit {
    fn drop(&mut self) {
        TASK_LIMIT.with(|value| value.set(self.0));
    }
}

#[test]
fn capture_runs_preserve_work_failures_before_joins_and_suppress_work_after_duplicate_joins() {
    let p = program();
    let compiled = super::capture_run::prepare(&p).unwrap();
    for source in [
        "f(1,2,3,\"end\")\n",
        "f(a,1,b,\"sep\",c,2,\"end\")\n",
        "f(a,1,b)\n",
    ] {
        let doc = tape(source);
        let root = root(&doc);
        for path in 0..=80 {
            for task in (0..=400).step_by(7) {
                let _budget = TaskLimit::enter(task);
                let expected = p.execute_roots_from_with_work_limit(&doc, [root], 0, None, path);
                let actual = compiled
                    .execute(&doc, root, Limits { path, task })
                    .map(|run| run.outputs);
                assert_eq!(
                    actual, expected,
                    "source={source:?} path={path} task={task}"
                );
            }
        }
    }
}

#[test]
fn capture_runs_match_the_interpreter_across_generated_mixed_sibling_domains() {
    let p = program();
    let compiled = super::capture_run::prepare(&p).unwrap();
    for length in 0..=6 {
        for code in 0..3usize.pow(length) {
            let mut code = code;
            let values = (0..length)
                .map(|index| {
                    let value = match code % 3 {
                        0 => format!("v{index}"),
                        1 => index.to_string(),
                        _ => format!("\"s{index}\""),
                    };
                    code /= 3;
                    value
                })
                .collect::<Vec<_>>();
            let source = format!("f({})\n", values.join(","));
            let doc = tape(&source);
            let root = root(&doc);
            let expected = p.execute_roots(&doc, [root]);
            let actual = compiled
                .execute(&doc, root, Limits::default())
                .map(|run| run.outputs);
            assert_eq!(actual, expected, "{source}");
        }
    }
}

#[test]
fn capture_runs_preserve_every_duplicate_capture_property_and_transformed_value() {
    let original = program();
    // Each body capture becomes two equal captures. Forward targets follow the
    // added instruction so this remains one independently verified transducer.
    let words = original
        .words
        .iter()
        .enumerate()
        .flat_map(|(pc, &w)| {
            let (op, mode, arg) = split(w);
            let adjusted = if matches!(op, FORK | GOTO) {
                word(
                    op,
                    mode,
                    arg + [6, 14]
                        .into_iter()
                        .filter(|&at| at < usize::from(arg))
                        .count() as u16,
                )
            } else {
                w
            };
            if [6, 14].contains(&pc) {
                vec![adjusted, w]
            } else {
                vec![adjusted]
            }
        })
        .collect();
    let p = Program::new(
        words,
        vec![
            b"prefix".to_vec(),
            b"body".to_vec(),
            b"language".to_vec(),
            b"example".to_vec(),
        ],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_directives(vec![
        Directive::Set {
            key: 2,
            value: Some(3),
        },
        Directive::SetCapture {
            capture: 1,
            key: 2,
            value: Some(3),
        },
        Directive::Downcase { capture: 1 },
    ])
    .unwrap();
    let doc = tape("f(A,1,B,\"sep\",C,2,\"end\")\n");
    let root = root(&doc);
    let mut expected = Vec::new();
    for (ordinal, names) in [
        (0, vec![b"A".as_slice(), b"A", b"B", b"B"]),
        (4, vec![b"C".as_slice(), b"C"]),
    ] {
        let mut captures = vec![capture(&doc, 0, b"(")];
        captures.extend(names.into_iter().map(|name| {
            let mut c = capture(&doc, 1, name);
            c.text = Some(name.to_ascii_lowercase());
            c.properties = vec![(2, Some(3))];
            c
        }));
        expected.push(Output {
            task_ordinal: 0,
            local_ordinal: ordinal,
            start: root,
            label: 0,
            captures,
            properties: vec![(2, Some(3))],
            dynamic_properties: vec![],
        });
    }
    assert_eq!(p.execute_roots(&doc, [root]).unwrap(), expected);
    let compiled = super::capture_run::prepare(&p).unwrap();
    let run = compiled.execute(&doc, root, Limits::default()).unwrap();
    assert_eq!(run.outputs, expected);
    assert_eq!(
        execute_with_backend(ExecutionBackend::CpuJit, &[p], &[doc], &[vec![vec![root]]]).unwrap(),
        vec![vec![expected]]
    );
}

#[test]
fn capture_runs_reject_invalid_programs_roots_and_limits_without_default_results() {
    let p = program();
    let compiled = super::capture_run::prepare(&p).unwrap();
    let doc = tape("f(a,\"end\")\n");
    for (root, limits) in [
        (u32::MAX, Limits::default()),
        (
            root(&doc),
            Limits {
                path: MAX_STEPS_PER_PATH + 1,
                task: MAX_WORK_PER_TASK,
            },
        ),
        (
            root(&doc),
            Limits {
                path: MAX_STEPS_PER_PATH,
                task: MAX_WORK_PER_TASK + 1,
            },
        ),
    ] {
        assert!(matches!(
            compiled.execute(&doc, root, limits),
            Err(Error::InvalidAnchor)
        ));
    }
    let mut invalid = p.clone();
    invalid.words[7] = word(GOTO, 0, u16::MAX);
    assert_eq!(
        super::capture_run::prepare(&invalid).err(),
        invalid.validate().err()
    );
    assert!(matches!(invalid.validate(), Err(Error::InvalidTarget(7))));
    let mut overlapping = p.clone();
    for pc in [8, 16] {
        overlapping.words[pc] = p.words[5];
    }
    overlapping.validate().unwrap();
    assert!(
        !overlapping
            .execute_roots(&doc, [root(&doc)])
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        super::capture_run::prepare(&overlapping),
        Err(Error::UnsupportedBackend)
    ));
    let mut ordinary = p.clone();
    ordinary.tree_sitter_query = false;
    ordinary.validate().unwrap();
    assert!(matches!(
        super::capture_run::prepare(&ordinary),
        Err(Error::UnsupportedBackend)
    ));
}

#[test]
fn capture_runs_preserve_aggregate_work_at_the_exact_last_admitted_operation() {
    let p = program();
    let compiled = super::capture_run::prepare(&p).unwrap();
    for source in [
        "f(1,2,3,\"end\")\n",
        "f(a,1,b,\"sep\",c,2,\"end\")\n",
        "f(a,1,b)\n",
    ] {
        let doc = tape(source);
        let root = root(&doc);
        let run = compiled.execute(&doc, root, Limits::default()).unwrap();
        let total = run.report.task_work;
        assert!(total > 0);
        for task in [total - 1, total, total + 1] {
            let _scope = TaskLimit::enter(task);
            let expected = p.execute_roots(&doc, [root]);
            if task < total {
                assert_eq!(expected, Err(Error::WorkLimit));
            } else {
                assert_eq!(expected, Ok(run.outputs.clone()));
            }
            assert_eq!(
                compiled
                    .execute(
                        &doc,
                        root,
                        Limits {
                            path: MAX_STEPS_PER_PATH,
                            task
                        }
                    )
                    .map(|r| r.outputs),
                expected
            );
        }
    }
}

#[test]
fn capture_runs_execute_a_compiled_scm_sibling_sequence() {
    let lang = SupportLang::Python.get_ts_language();
    let p = Program::compile_tree_sitter_query_pattern(
        "((identifier) @prefix [(identifier) @body (integer)]+ (string))",
        &lang,
    )
    .unwrap();
    let doc = tape("f(a,1,b,\"sep\",c,2,\"end\")\n");
    let root = capture(&doc, 0, b"a").node;
    let expected = p.execute_roots(&doc, [root]).unwrap();
    assert_eq!(
        expected.iter().map(|r| r.local_ordinal).collect::<Vec<_>>(),
        [2, 4]
    );
    let compiled = super::capture_run::prepare(&p).unwrap();
    assert_eq!(
        compiled
            .execute(&doc, root, Limits::default())
            .unwrap()
            .outputs,
        expected
    );
}

#[test]
fn capture_runs_keep_unnamed_repeat_steps_and_unrestored_tail_identity_at_work_boundaries() {
    let lang = SupportLang::Python.get_ts_language();
    let p = Program::compile_tree_sitter_query_pattern(
        "((identifier) @prefix [(identifier) @body (integer)]+ (string))",
        &lang,
    )
    .unwrap();
    let compiled = super::capture_run::prepare(&p).unwrap();
    for source in [
        "f(a,1,2,\"sep\",3,\"end\")\n",
        "f(a,b,c,\"sep\",d,e,\"end\")\n",
    ] {
        let doc = tape(source);
        let root = capture(&doc, 0, b"a").node;
        let complete = compiled.execute(&doc, root, Limits::default()).unwrap();
        assert_eq!(complete.outputs, p.execute_roots(&doc, [root]).unwrap());
        for path in 0..=60 {
            for task in (0..=complete.report.task_work + 1).step_by(7) {
                let _budget = TaskLimit::enter(task);
                assert_eq!(
                    compiled
                        .execute(&doc, root, Limits { path, task })
                        .map(|r| r.outputs),
                    p.execute_roots_from_with_work_limit(&doc, [root], 0, None, path),
                    "source={source:?} path={path} task={task}"
                );
            }
        }
        for task in [complete.report.task_work - 1, complete.report.task_work] {
            let _budget = TaskLimit::enter(task);
            let expected = p.execute_roots(&doc, [root]);
            if task < complete.report.task_work {
                assert_eq!(expected, Err(Error::WorkLimit));
            } else {
                assert_eq!(expected, Ok(complete.outputs.clone()));
            }
            assert_eq!(
                compiled
                    .execute(
                        &doc,
                        root,
                        Limits {
                            path: MAX_STEPS_PER_PATH,
                            task
                        }
                    )
                    .map(|r| r.outputs),
                expected
            );
        }
    }
}

#[test]
fn capture_runs_execute_long_compiled_string_repetition_with_one_event_arena() {
    let lang = SupportLang::Python.get_ts_language();
    let query = "((string_start) @prefix [(string_content) @body (interpolation)]+ (string_end))";
    let p = Program::compile_tree_sitter_query_pattern(query, &lang).unwrap();
    let compiled = super::capture_run::prepare(&p).unwrap();
    for parts in [1usize, 8, 350] {
        let source = format!("f\"a{}\"\n", "{value}a".repeat(parts));
        let doc = tape(&source);
        let root = (0..doc.transducer_len() as u32)
            .find(|&n| doc.transducer_kind(n) == lang.id_for_node_kind("string_start", true))
            .unwrap();
        let label = |name: &[u8]| {
            p.labels
                .iter()
                .position(|&i| p.strings[i as usize] == name)
                .unwrap() as u16
        };
        let mut captures = vec![Capture {
            label: label(b"prefix"),
            node: root,
            span: 0..2,
            text: None,
            properties: vec![],
        }];
        for part in 0..=parts {
            let span = 2 + part * 8..3 + part * 8;
            let nodes = (0..doc.transducer_len() as u32)
                .filter(|&node| doc.transducer_span(node) == span)
                .collect::<Vec<_>>();
            assert_eq!(nodes.len(), 1);
            captures.push(Capture {
                label: label(b"body"),
                node: nodes[0],
                span,
                text: None,
                properties: vec![],
            });
        }
        let expected = vec![Output {
            task_ordinal: 0,
            local_ordinal: 0,
            start: root,
            label: label(b"tree-sitter-query"),
            captures,
            properties: vec![],
            dynamic_properties: vec![],
        }];
        if parts <= 8 {
            assert_eq!(p.execute_roots(&doc, [root]).unwrap(), expected);
        }
        let started = std::time::Instant::now();
        let run = compiled.execute(&doc, root, Limits::default()).unwrap();
        eprintln!(
            "CAPTURE_RUN_LONG parts={parts} seconds={} report={:?}",
            started.elapsed().as_secs_f64(),
            run.report
        );
        assert_eq!(run.outputs, expected);
        assert_eq!(run.report.capture_events, parts + 1);
        assert_eq!(run.report.materialized_capture_records, parts + 2);
        assert!(run.report.task_work <= MAX_WORK_PER_TASK);
        if parts == 350 {
            assert!(run.report.exit_records > 1000);
        }
    }
}

#[test]
fn capture_runs_preserve_all_initial_repeat_and_tail_navigation_modes() {
    for modes in 0..8 {
        let mut p = program();
        p.words[3] = word(FOLLOWING, u8::from(modes & 1 != 0), 0);
        p.words[11] = word(NEXT, u8::from(modes & 2 != 0), 0);
        p.words[20] = word(FOLLOWING, u8::from(modes & 4 != 0), 0);
        p = p
            .with_capture_predicates(vec![CapturePredicate::Kind {
                capture: 1,
                kind: SupportLang::Python
                    .get_ts_language()
                    .id_for_node_kind("identifier", true),
                negated: false,
                any: false,
            }])
            .unwrap();
        // ASSERT is deferred for query execution. After RESTORE its admission
        // is shared, so duplicate tails must not execute it again.
        p.words.insert(23, word(ASSERT, 0, 0));
        p.validate().unwrap();
        let compiled = super::capture_run::prepare(&p).unwrap();
        for source in [
            "f(1,2,3,\"end\")\n",
            "f(a,1,b,\"sep\",c,2,\"end\")\n",
            "f(a,1,b)\n",
        ] {
            let doc = tape(source);
            let root = root(&doc);
            let run = compiled.execute(&doc, root, Limits::default()).unwrap();
            assert_eq!(
                run.outputs,
                p.execute_roots(&doc, [root]).unwrap(),
                "modes={modes} source={source:?}"
            );
            for path in 0..=80 {
                assert_eq!(
                    compiled
                        .execute(
                            &doc,
                            root,
                            Limits {
                                path,
                                task: MAX_WORK_PER_TASK
                            }
                        )
                        .map(|r| r.outputs),
                    p.execute_roots_from_with_work_limit(&doc, [root], 0, None, path),
                    "modes={modes} path={path}"
                );
            }
            for task in [run.report.task_work - 1, run.report.task_work] {
                let _scope = TaskLimit::enter(task);
                let expected = p.execute_roots(&doc, [root]);
                if task < run.report.task_work {
                    assert_eq!(expected, Err(Error::WorkLimit));
                } else {
                    assert_eq!(expected, Ok(run.outputs.clone()));
                }
                assert_eq!(
                    compiled
                        .execute(
                            &doc,
                            root,
                            Limits {
                                path: MAX_STEPS_PER_PATH,
                                task
                            }
                        )
                        .map(|r| r.outputs),
                    expected,
                    "modes={modes} task={task}"
                );
            }
        }
    }
}

#[test]
fn capture_runs_refuse_capture_ranges_that_skip_an_observable_unnamed_event() {
    let mut p = program();
    let lang = SupportLang::Python.get_ts_language();
    p.words[3] = word(FOLLOWING, 0, 0);
    for pc in [5, 13] {
        p.words[pc] = word(KIND, 0, lang.id_for_node_kind(",", false));
    }
    p.validate().unwrap();
    let doc = tape("f(a,1,b,\"end\")\n");
    let root = root(&doc);
    let expected = p.execute_roots(&doc, [root]).unwrap();
    assert!(!expected.is_empty());
    assert!(matches!(
        super::capture_run::prepare(&p)
            .unwrap()
            .execute(&doc, root, Limits::default()),
        Err(Error::UnsupportedBackend)
    ));
}

#[test]
#[ignore = "isolated warmed CPU interpreter and capture-run execution measurements"]
fn capture_run_execution_benchmark() {
    let p = program();
    let compiled = super::capture_run::prepare(&p).unwrap();
    for parts in [8, 32, 64] {
        let source = format!(
            "f({},\"end\")\n",
            (0..parts)
                .flat_map(|i| [format!("v{i}"), i.to_string()])
                .collect::<Vec<_>>()
                .join(",")
        );
        let doc = tape(&source);
        let root = root(&doc);
        let expected = p.execute_roots(&doc, [root]).unwrap();
        assert_eq!(
            compiled
                .execute(&doc, root, Limits::default())
                .unwrap()
                .outputs,
            expected
        );
        let mut reference = Vec::new();
        let mut capture_run = Vec::new();
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let actual = p.execute_roots(std::hint::black_box(&doc), [root]).unwrap();
            reference.push(started.elapsed().as_secs_f64());
            assert_eq!(actual, expected);
            let started = std::time::Instant::now();
            let actual = compiled
                .execute(std::hint::black_box(&doc), root, Limits::default())
                .unwrap();
            capture_run.push(started.elapsed().as_secs_f64());
            assert_eq!(actual.outputs, expected);
        }
        eprintln!(
            "CAPTURE_RUN_BENCHMARK {}",
            serde_json::json!({"parts":parts,"sourceBytes":source.len(),"referenceSeconds":reference,"captureRunSeconds":capture_run,"captures":expected[0].captures.len(),"profile":"debug","warmup":true,"parseAndPreparationExcluded":true})
        );
    }
}
