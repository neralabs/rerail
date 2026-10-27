//! Whole-value and complete-evaluation resource proof for finite literal queries.
use super::super::*;
use crate::policy_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::{LanguageExt, SupportLang};

fn program(mode: usize) -> Program {
    let predicate = if mode & 8 == 0 {
        TextPredicate::Equal(3)
    } else {
        TextPredicate::Prefix(3)
    };
    let literal = if mode >= 16 || mode & 8 == 0 {
        b"keep".to_vec()
    } else {
        b"ke".to_vec()
    };
    Program::new(
        vec![
            word(
                KIND,
                0,
                SupportLang::Python
                    .get_ts_language()
                    .id_for_node_kind("identifier", true),
            ),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 2),
        ],
        vec![
            b"name".to_vec(),
            b"missing".to_vec(),
            b"match".to_vec(),
            literal,
            b"kind".to_vec(),
            b"kept".to_vec(),
        ],
        vec![predicate],
        vec![0, 1, 2],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_capture_predicates(vec![if mode >= 16 {
        CapturePredicate::AnyOf {
            capture: u16::from(mode & 4 != 0),
            strings: if mode & 8 == 0 { vec![3, 5, 3] } else { vec![] },
            negated: mode & 1 != 0,
            any: mode & 2 != 0,
        }
    } else {
        CapturePredicate::Text {
            capture: u16::from(mode & 4 != 0),
            predicate: 0,
            negated: mode & 1 != 0,
            any: mode & 2 != 0,
        }
    }])
    .unwrap()
    .with_directives(vec![
        Directive::Set {
            key: 4,
            value: Some(5),
        },
        Directive::SetCapture {
            capture: 0,
            key: 4,
            value: Some(5),
        },
    ])
    .unwrap()
}
fn tape() -> TapeDoc {
    tape_source("skip\nkeep\n".repeat(50))
}
fn tape_source(source: String) -> TapeDoc {
    let source = Arc::new(source);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    TapeDoc::restore(source, grammar, &image, &|| false).unwrap()
}

#[test]
fn finite_literal_columns_keep_raw_ordinal_gaps_after_rejected_first_alternatives() {
    check_raw_ordinal_gap(r#"(#eq? @v "keep")"#);
    check_raw_ordinal_gap(r#"(#any-of? @v "keep" "unused" "keep")"#);
    check_raw_ordinal_gap(r#"(#match? @v "^k")"#);
    check_raw_ordinal_gap(r#"(#lua-match? @v "^k")"#);
}

fn check_raw_ordinal_gap(predicate: &str) {
    use super::super::metal_grid::MetalGridInput;
    use super::super::metal_grid_executor::GuardedExecutor;
    use super::super::metal_grid_plan::{ExecutionPlan, ResidentInput};
    let p = Program::compile_tree_sitter_query_pattern(
        &r#"([(call function: (identifier) @v)
        (call arguments: (argument_list (identifier) @v))]
        (#eq? @v "keep") (#set! role "kept"))"#
            .replace(r#"(#eq? @v "keep")"#, predicate),
        &SupportLang::Python.get_ts_language(),
    )
    .unwrap();
    let doc = tape_source(format!("{}skip(keep)\n", "skip\n".repeat(20_000)));
    let kind = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("identifier", true);
    let roots = (0..doc.transducer_len() as u32)
        .filter(|&n| doc.transducer_kind(n) == kind)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 20_002);
    let call_kind = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("call", true);
    let calls = (0..doc.transducer_len() as u32)
        .filter(|&n| doc.transducer_kind(n) == call_kind)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let name = (0..doc.transducer_len() as u32)
        .filter(|&n| doc.transducer_kind(n) == kind && doc.transducer_text(n) == b"keep")
        .collect::<Vec<_>>();
    assert_eq!(name.len(), 1);
    let label = |name: &[u8]| {
        p.labels
            .iter()
            .position(|&i| p.strings[i as usize] == name)
            .unwrap() as u16
    };
    let string = |name: &[u8]| p.strings.iter().position(|s| s == name).unwrap() as u16;
    let wanted = vec![Output {
        task_ordinal: 0,
        local_ordinal: 1,
        start: calls[0],
        label: label(b"tree-sitter-query"),
        captures: vec![Capture {
            label: label(b"v"),
            node: name[0],
            span: doc.transducer_span(name[0]),
            text: None,
            properties: vec![],
        }],
        properties: vec![(string(b"role"), Some(string(b"kept")))],
        dynamic_properties: vec![],
    }];
    assert_eq!(
        execute_batch(
            std::slice::from_ref(&p),
            std::slice::from_ref(&doc),
            Some(&[vec![calls.clone()]])
        )
        .unwrap(),
        vec![vec![wanted.clone()]]
    );
    let mut programs = vec![program(0); 200];
    programs.push(p);
    let mut anchors = vec![Arc::<[u32]>::from(roots.clone()); 200];
    anchors.push(Arc::from(calls));
    let resident = ResidentInput::new(
        MetalGridInput::build_shared(&programs, std::slice::from_ref(&doc), &[anchors]).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let actual = GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute()
        .unwrap()
        .into_outcome()
        .unwrap()
        .into_result()
        .unwrap();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].len(), 201);
    for (i, outputs) in actual[0][..200].iter().enumerate() {
        let mut want = expected(&doc, 0, &roots);
        for row in &mut want {
            row.task_ordinal += i as u32 * roots.len() as u32;
        }
        assert_eq!(outputs, &want);
    }
    let mut wanted = wanted;
    wanted[0].task_ordinal = 200 * roots.len() as u32;
    assert_eq!(
        actual[0][200], wanted,
        "rejected ordinal zero must remain a gap in final literal columns"
    );
}
fn expected(doc: &TapeDoc, mode: usize, roots: &[u32]) -> Vec<Output> {
    roots
        .iter()
        .enumerate()
        .filter(|&(_, &root)| {
            let matched = !(mode >= 16 && mode & 8 != 0) && doc.transducer_text(root) == b"keep";
            mode & 4 == 0 && matched == (mode & 1 == 0)
        })
        .map(|(task, &root)| Output {
            task_ordinal: task as u32,
            local_ordinal: 0,
            start: root,
            label: 2,
            captures: vec![
                Capture {
                    label: 0,
                    node: root,
                    span: doc.transducer_span(root),
                    text: None,
                    properties: vec![(4, Some(5))]
                };
                2
            ],
            properties: vec![(4, Some(5))],
            dynamic_properties: vec![],
        })
        .collect()
}

#[test]
fn finite_literal_queries_preserve_complete_output_and_fit_one_evaluation_without_raw_candidates() {
    complete_literal_evaluation(0);
}

#[test]
fn finite_literal_sets_preserve_complete_output_and_fit_one_evaluation_without_raw_candidates() {
    complete_literal_evaluation(16);
}

fn complete_literal_evaluation(base: usize) {
    use super::super::metal_backend::launch::LaunchScope;
    use super::super::metal_grid::MetalGridInput;
    use super::super::metal_grid_executor::GuardedExecutor;
    use super::super::metal_grid_plan::{ExecutionPlan, ResidentInput};
    use super::super::metal_output_pages::PageBounds;
    let doc = tape();
    let identifier = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("identifier", true);
    let roots = (0..doc.transducer_len() as u32)
        .filter(|&n| {
            doc.transducer_kind(n) == identifier
                && (doc.transducer_text(n) == b"skip" || doc.transducer_text(n) == b"keep")
        })
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 100);
    for mode in base..base + 16 {
        let oracle = execute_batch(
            &[program(mode)],
            std::slice::from_ref(&doc),
            Some(&[vec![roots.clone()]]),
        )
        .unwrap()
        .pop()
        .unwrap()
        .pop()
        .unwrap();
        assert_eq!(
            oracle,
            expected(&doc, mode, &roots),
            "independent CPU truth mode={mode}"
        );
    }
    let programs = (0..1500)
        .map(|i| program(base + i % 16))
        .collect::<Vec<_>>();
    let files = vec![doc.clone(); 100];
    let roots: Arc<[u32]> = roots.into();
    let anchors = vec![vec![Arc::clone(&roots); programs.len()]; files.len()];
    let resident = ResidentInput::new(
        MetalGridInput::build_shared(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let started = std::time::Instant::now();
    let mut completed = 0;
    let mut matched = 0usize;
    let result = GuardedExecutor::new(resident, plan)
        .unwrap()
        .with_output_bounds(PageBounds {
            rows: 128 * 1024,
            captures: 256 * 1024,
            properties: 512 * 1024,
            bytes: 64 * 1024 * 1024,
        })
        .execute_pages(&mut |page, end| {
            assert!(end > completed && end <= files.len());
            for file in completed..end {
                for i in 0..programs.len() {
                    let (actual, error) = page.program_results(file, i)?;
                    assert_eq!(error, None);
                    let wanted = expected(&doc, base + i % 16, &roots);
                    assert_eq!(actual.len(), wanted.len(), "file={file} program={i}");
                    for (row, wanted) in actual.into_iter().zip(wanted) {
                        assert_eq!(
                            row.task_ordinal(),
                            i as u32 * roots.len() as u32 + wanted.task_ordinal
                        );
                        assert_eq!(
                            (row.local_ordinal(), row.start_node(), row.label()),
                            (wanted.local_ordinal, wanted.start, wanted.label)
                        );
                        let captures = row
                            .captures()
                            .map(|c| Capture {
                                label: c.label(),
                                node: c.node(),
                                span: c.span(),
                                text: c.transformed_text().map(<[u8]>::to_vec),
                                properties: c.properties().collect(),
                            })
                            .collect::<Vec<_>>();
                        assert_eq!(captures, wanted.captures);
                        assert_eq!(row.properties().collect::<Vec<_>>(), wanted.properties);
                        assert_eq!(row.dynamic_properties().count(), 0);
                        matched += 1;
                    }
                }
            }
            completed = end;
            Ok(())
        });
    let trace = scope.snapshot();
    eprintln!(
        "FINITE_LITERAL_CLASS_EVIDENCE {}",
        serde_json::json!({"programs":programs.len(),"files":files.len(),"logicalRoots":15000000,"matches":matched,"seconds":started.elapsed().as_secs_f64(),"dispatches":trace.len(),"error":format!("{result:?}"),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
    );
    assert_eq!(
        result,
        Ok(()),
        "the complete literal-query evaluation must fit the original memory and launch guards"
    );
    assert_eq!(completed, files.len());
    let want = (0..programs.len())
        .map(|i| expected(&doc, base + i % 16, &roots).len())
        .sum::<usize>()
        * files.len();
    assert_eq!(matched, want);
    assert!(matched > 1_000_000);
    assert!(!trace.is_empty() && trace.len() <= 600);
    assert!(trace.iter().all(|r| r.encoded
        && r.submitted
        && r.completed
        && r.peak_application_buffer_bytes <= 2_000_000_000));
}
