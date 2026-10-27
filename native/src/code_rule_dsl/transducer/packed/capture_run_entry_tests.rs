use super::capture_run::Limits;
use super::*;
use crate::policy_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::{LanguageExt, SupportLang};

const QUERY: &str = r#"((call function: (attribute object: (identifier) @_re)
  arguments: (argument_list (string (string_start)
    [(string_content) @injection.content (interpolation)]+ (string_end))))
  (#eq? @_re "re") (#set! injection.language "regex"))"#;

fn program() -> Program {
    Program::compile_tree_sitter_query_pattern(QUERY, &SupportLang::Python.get_ts_language())
        .unwrap()
}

pub(super) fn tape(source: &str) -> TapeDoc {
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}

fn calls(doc: &TapeDoc) -> Vec<u32> {
    let kind = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("call", true);
    (0..doc.transducer_len() as u32)
        .filter(|&node| doc.transducer_kind(node) == kind)
        .collect()
}

#[test]
fn capture_runs_bind_complete_outer_frames_prefix_captures_and_deferred_values() {
    let p = program();
    for parts in [1usize, 8, 350] {
        let source = format!(
            "import re\npattern = re.compile(f\"a{}\")\n",
            "{value}a".repeat(parts)
        );
        let doc = tape(&source);
        let roots = calls(&doc);
        assert_eq!(roots.len(), 1);
        let string =
            |name: &[u8]| u16::try_from(p.strings.iter().position(|s| s == name).unwrap()).unwrap();
        let label = |name: &[u8]| {
            u16::try_from(
                p.labels
                    .iter()
                    .position(|&s| p.strings[s as usize] == name)
                    .unwrap(),
            )
            .unwrap()
        };
        let start = source.find("re.compile").unwrap();
        let content = source.find("f\"a").unwrap() + 2;
        let spans =
            std::iter::once((label(b"_re"), start..start + 2)).chain((0..=parts).map(|at| {
                (
                    label(b"injection.content"),
                    content + at * 8..content + at * 8 + 1,
                )
            }));
        let captures = spans
            .map(|(label, span)| {
                let nodes = (0..doc.transducer_len() as u32)
                    .filter(|&node| doc.transducer_span(node) == span)
                    .collect::<Vec<_>>();
                assert_eq!(nodes.len(), 1, "capture span {span:?}");
                Capture {
                    label,
                    node: nodes[0],
                    span,
                    text: None,
                    properties: vec![],
                }
            })
            .collect();
        let expected = vec![Output {
            task_ordinal: 0,
            local_ordinal: 0,
            start: roots[0],
            label: label(b"tree-sitter-query"),
            captures,
            properties: vec![(string(b"injection.language"), Some(string(b"regex")))],
            dynamic_properties: vec![],
        }];
        if parts <= 8 {
            assert_eq!(
                p.execute_roots(&doc, [roots[0]]).unwrap(),
                expected,
                "independent interpreter must establish the complete outer fixture first"
            );
        }
        let prepared = capture_run::prepare(&p).unwrap();
        let started = std::time::Instant::now();
        let result = prepared.execute(&doc, roots[0], Limits::default()).unwrap();
        eprintln!(
            "OUTER_CAPTURE_RUN parts={parts} seconds={} report={:?}",
            started.elapsed().as_secs_f64(),
            result.report
        );
        assert_eq!(result.outputs, expected, "parts={parts}");
        assert_eq!(result.report.materialized_capture_records, parts + 2);
    }
}

#[test]
fn outer_capture_runs_preserve_all_results_and_exact_work_boundaries_across_prefix_branches() {
    let p = program();
    let prepared = capture_run::prepare(&p).unwrap();
    for source in [
        "re.compile(f\"a{x}b\")\n",
        "re.compile(f\"a{x}b\", f\"c{y}d\")\n",
        "re.compile(f\"{x}{y}\", f\"{a}{b}\")\n",
        "other.compile(f\"a{x}b\")\n",
        "re.compile(\"x\", f\"a{value}b\", f\"{other}\")\n",
        "re.compile(f\"a{x}b{y}c\")\n",
        "re.sub.compile(f\"a{x}b\")\n",
        "re.compile()\n",
    ] {
        let doc = tape(source);
        let roots = calls(&doc);
        assert_eq!(roots.len(), 1);
        let root = roots[0];
        let full = prepared.execute(&doc, root, Limits::default()).unwrap();
        assert_eq!(
            full.outputs,
            p.execute_roots(&doc, [root]).unwrap(),
            "source={source}"
        );
        let total = full.report.task_work;
        for task in [total - 1, total, total + 1] {
            let _limit = capture_run_tests::TaskLimit::enter(task);
            let expected = p.execute_roots(&doc, [root]);
            let actual = prepared
                .execute(
                    &doc,
                    root,
                    Limits {
                        path: MAX_STEPS_PER_PATH,
                        task,
                    },
                )
                .map(|run| run.outputs);
            assert_eq!(
                actual, expected,
                "source={source} task={task} exact={total}"
            );
            assert_eq!(
                actual.is_err(),
                task < total,
                "exact aggregate boundary source={source}"
            );
        }
        for path in 0..=100 {
            for task in (0..=500).step_by(19) {
                let _limit = capture_run_tests::TaskLimit::enter(task);
                let expected = p.execute_roots_from_with_work_limit(&doc, [root], 0, None, path);
                let actual = prepared
                    .execute(&doc, root, Limits { path, task })
                    .map(|run| run.outputs);
                assert_eq!(actual, expected, "source={source} path={path} task={task}");
            }
        }
    }
}

#[test]
fn outer_prefix_cursor_truncation_rejects_reconvergence_and_unbound_restores() {
    let language = SupportLang::Python.get_ts_language();
    let p = Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(SAVE_CURSOR, 0, 0),
            word(CHILD, NAMED, 0),
            word(SAVE_CURSOR, 0, 7),
            word(RESTORE_CURSOR, 0, 0),
            word(SAVE_CURSOR, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(FOLLOWING, NAMED, 0),
            word(KIND, 0, language.id_for_node_kind("identifier", true)),
            word(CAPTURE, 0, 1),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, 0, 17),
            word(NEXT, NAMED, 0),
            word(KIND, 0, language.id_for_node_kind("identifier", true)),
            word(CAPTURE, 0, 1),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 11),
            word(REPEAT_END, 0, 0),
            word(FOLLOWING, NAMED, 0),
            word(KIND, 0, language.id_for_node_kind("string", true)),
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
    .unwrap();
    let doc = tape("f([a,b],x,\"end\")\n");
    let root = (0..doc.transducer_len() as u32)
        .find(|&n| doc.transducer_kind(n) == language.id_for_node_kind("list", true))
        .unwrap();
    let body = (0..doc.transducer_len() as u32)
        .filter(|&n| doc.transducer_text(n) == b"x")
        .collect::<Vec<_>>();
    assert_eq!(body.len(), 1);
    let expected = vec![Output {
        task_ordinal: 0,
        local_ordinal: 0,
        start: root,
        label: 0,
        captures: vec![
            Capture {
                label: 0,
                node: root,
                span: 2..7,
                text: None,
                properties: vec![],
            },
            Capture {
                label: 1,
                node: body[0],
                span: 8..9,
                text: None,
                properties: vec![],
            },
        ],
        properties: vec![],
        dynamic_properties: vec![],
    }];
    assert_eq!(p.execute_roots(&doc, [root]).unwrap(), expected);
    let prepared = capture_run::prepare(&p).unwrap();
    assert_eq!(
        prepared
            .execute(&doc, root, Limits::default())
            .map(|run| run.outputs),
        Err(Error::UnsupportedBackend),
        "saving slot zero erases slot seven; the resulting prefix join requires general admission"
    );

    let mut unbound = p;
    unbound.words[20] = word(RESTORE_CURSOR, 0, 7);
    unbound.validate().unwrap();
    assert_eq!(
        unbound.execute_roots(&doc, [root]),
        Err(Error::InvalidOperand(20))
    );
    assert!(
        matches!(
            capture_run::prepare(&unbound),
            Err(Error::UnsupportedBackend)
        ),
        "a truncated cursor cannot prove a bound suffix restore"
    );
}

#[test]
fn streaming_outer_capture_runs_preserve_complete_values_raw_counts_and_semantic_work() {
    let p = program();
    let prepared = capture_run::prepare(&p).unwrap();
    for parts in [0usize, 1, 8, 350] {
        let source = format!("re.compile(f\"a{}\")\n", "{value}a".repeat(parts));
        let doc = tape(&source);
        let root = calls(&doc)[0];
        let expected = prepared.execute(&doc, root, Limits::default()).unwrap();
        if parts <= 8 {
            assert_eq!(expected.outputs, p.execute_roots(&doc, [root]).unwrap());
        }
        let started = std::time::Instant::now();
        let actual = prepared
            .execute_streaming(&doc, root, Limits::default())
            .unwrap();
        eprintln!(
            "STREAMING_OUTER_CAPTURE_RUN parts={parts} seconds={}",
            started.elapsed().as_secs_f64()
        );
        assert_eq!(actual.outputs, expected.outputs, "parts={parts}");
        assert_eq!(
            actual.report, expected.report,
            "exact work and candidate accounting parts={parts}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn metal_outer_capture_runs_bound_generations_and_preserve_complete_values() {
    let p = program();
    let parts = 129;
    let source = format!("re.compile(f\"a{}\")\n", "{value}a".repeat(parts));
    let doc = tape(&source);
    let root = calls(&doc)[0];
    let label = |name: &[u8]| {
        u16::try_from(
            p.labels
                .iter()
                .position(|&s| p.strings[s as usize] == name)
                .unwrap(),
        )
        .unwrap()
    };
    let string =
        |name: &[u8]| u16::try_from(p.strings.iter().position(|s| s == name).unwrap()).unwrap();
    let content = source.find("f\"a").unwrap() + 2;
    let captures = std::iter::once((label(b"_re"), 0..2))
        .chain((0..=parts).map(|at| {
            (
                label(b"injection.content"),
                content + at * 8..content + at * 8 + 1,
            )
        }))
        .map(|(label, span)| {
            let nodes = (0..doc.transducer_len() as u32)
                .filter(|&node| doc.transducer_span(node) == span)
                .collect::<Vec<_>>();
            assert_eq!(nodes.len(), 1, "capture span {span:?}");
            Capture {
                label,
                node: nodes[0],
                span,
                text: None,
                properties: vec![],
            }
        })
        .collect();
    let expected = vec![Output {
        task_ordinal: 0,
        local_ordinal: 0,
        start: root,
        label: label(b"tree-sitter-query"),
        captures,
        properties: vec![(string(b"injection.language"), Some(string(b"regex")))],
        dynamic_properties: vec![],
    }];
    let prepared = capture_run::prepare(&p).unwrap();
    assert_eq!(
        prepared
            .execute_streaming(&doc, root, Limits::default())
            .unwrap()
            .outputs,
        expected
    );
    let scope = MetalTestScope::enter();
    let started = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, &[p], &[doc], &[vec![vec![root]]]);
    let trace = scope.snapshot();
    let generations = trace
        .iter()
        .filter(|r| r.kernel == "successor_inputs_checked")
        .count();
    eprintln!(
        "OUTER_METAL_RUN parts={parts} generations={generations} dispatches={} seconds={} peakMetalBytes={:?}",
        trace.len(),
        started.elapsed().as_secs_f64(),
        trace.iter().map(|r| r.peak_application_buffer_bytes).max()
    );
    assert_eq!(actual, Ok(vec![vec![expected]]));
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        trace.len() <= 260,
        "complete evaluation exceeds 260 dispatches: {}",
        trace.len()
    );
    assert!(
        generations > 0 && generations <= 10,
        "complete capture repetition needs at most ten bounded successor generations, got {generations}"
    );
}
